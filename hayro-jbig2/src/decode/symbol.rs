//! Symbol dictionary segment parsing and decoding (7.4.2, 6.5).

use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::decode::generic;
use crate::decode::generic::{decode_bitmap_mmr, parse_adaptive_template_pixels};
use crate::decode::generic_refinement::decode_bitmap as decode_refinement_bitmap;
use crate::decode::text::{
    ReferenceCorner, TextRegionContexts, TextRegionParams, decode_text_region_refine_with_contexts,
};
use crate::decode::{
    AdaptiveTemplatePixel, CombinationOperator, RefinementTemplate, Template,
    parse_refinement_at_pixels,
};
use crate::error::{DecodeError, HuffmanError, ParseError, RegionError, Result, SymbolError, bail};
use crate::huffman_table::{HuffmanTable, StandardHuffmanTables};
use crate::integer_decoder::IntegerDecoder;
use crate::reader::Reader;

/// Decode a symbol dictionary segment (7.4.2, 6.5).
pub(crate) fn decode(
    reader: &mut Reader<'_>,
    input_symbols: &[&DecodedRegion],
    referred_tables: &[HuffmanTable],
    standard_tables: &StandardHuffmanTables,
) -> Result<SymbolDictionary> {
    let header = parse(reader)?;

    let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    let mut arithmetic_context = ArithmeticContext::new(data, &header);
    let mut huffman_context = HuffmanContext::new(data, &header, referred_tables, standard_tables)?;

    let read_height_delta = |h_ctx: &mut HuffmanContext<'_>, a_ctx: &mut ArithmeticContext<'_>| {
        if header.flags.use_huffman {
            huffman_context.delta_height_table.decode(&mut h_ctx.reader)
        } else {
            Ok(a_ctx.delta_height_decoder.decode(&mut a_ctx.decoder))
        }
    };

    let read_width_delta = |h_ctx: &mut HuffmanContext<'_>, a_ctx: &mut ArithmeticContext<'_>| {
        if header.flags.use_huffman {
            huffman_context.delta_width_table.decode(&mut h_ctx.reader)
        } else {
            Ok(a_ctx.delta_width_decoder.decode(&mut a_ctx.decoder))
        }
    };

    let decode_symbol_run_length =
        |h_ctx: &mut HuffmanContext<'_>, a_ctx: &mut ArithmeticContext<'_>| {
            if header.flags.use_huffman {
                h_ctx.symbol_run_length_table.decode(&mut h_ctx.reader)
            } else {
                Ok(a_ctx.symbol_run_length_decoder.decode(&mut a_ctx.decoder))
            }
        };

    let num_new_symbols = header.num_new_symbols;

    let mut new_symbols = Vec::with_capacity(num_new_symbols as usize);
    // Only used if SDHUFF = 1 and SDREFAGG = 0.
    let mut symbol_widths = Vec::with_capacity(num_new_symbols as usize);

    let mut hcheight: u32 = 0;
    let mut nsymsdecoded: u32 = 0;

    while nsymsdecoded < num_new_symbols {
        let hcdh = read_height_delta(&mut huffman_context, &mut arithmetic_context)?
            .ok_or(SymbolError::OutOfRange)?;

        hcheight = hcheight
            .checked_add_signed(hcdh)
            .ok_or(RegionError::InvalidDimension)?;

        let mut symwidth: u32 = 0;
        let mut totwidth: u32 = 0;
        let hcfirstsym = nsymsdecoded;

        // "If the result of this decoding is OOB then all the symbols
        // in this height class have been decoded."
        while let Some(dw) = read_width_delta(&mut huffman_context, &mut arithmetic_context)? {
            symwidth = symwidth
                .checked_add_signed(dw)
                .ok_or(RegionError::InvalidDimension)?;
            totwidth = totwidth
                .checked_add(symwidth)
                .ok_or(RegionError::InvalidDimension)?;

            match (header.flags.use_huffman, header.flags.use_refagg) {
                (false, false) => {
                    let mut region = DecodedRegion::new(symwidth, hcheight);
                    generic::decode_bitmap_arithmetic_coding(
                        &mut region,
                        &mut arithmetic_context.decoder,
                        &mut arithmetic_context.bitmap_decode_contexts,
                        header.flags.template,
                        false,
                        &header.adaptive_template_pixels,
                    )?;

                    new_symbols.push(region);
                }
                (true, false) => {
                    // Decode a single symbol width. We don't actually decode the symbols
                    // yet, those will be decoded later on from the collective bitmap.
                    symbol_widths.push(symwidth);
                }
                (_, true) => {
                    let symbol = decode_bitmap_refagg(
                        &header,
                        &mut arithmetic_context,
                        &mut huffman_context,
                        input_symbols,
                        &new_symbols,
                        symwidth,
                        hcheight,
                        standard_tables,
                    )?;

                    new_symbols.push(symbol);
                }
            }

            nsymsdecoded += 1;
        }

        if header.flags.use_huffman && !header.flags.use_refagg {
            // Now, we use the symbol widths to decode the collective bitmap.
            decode_height_class_collective_bitmap(
                &mut huffman_context.reader,
                huffman_context.bitmap_size_table,
                &mut new_symbols,
                &symbol_widths,
                hcfirstsym,
                nsymsdecoded,
                totwidth,
                hcheight,
            )?;
        }
    }

    let num_input_symbols = input_symbols.len() as u32;

    let exported = decode_exported_symbols_with(
        num_input_symbols,
        header.num_exported_symbols,
        input_symbols,
        &new_symbols,
        || decode_symbol_run_length(&mut huffman_context, &mut arithmetic_context),
    )?;

    Ok(SymbolDictionary {
        exported_symbols: exported,
    })
}

/// Decode a symbol bitmap using refinement/aggregate coding (6.5.8.2).
#[allow(clippy::too_many_arguments)]
fn decode_bitmap_refagg(
    header: &SymbolDictionaryHeader,
    a_ctx: &mut ArithmeticContext<'_>,
    h_ctx: &mut HuffmanContext<'_>,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    standard_tables: &StandardHuffmanTables,
) -> Result<DecodedRegion> {
    // 6.5.8.2.1 Number of symbol instances in aggregation.
    let refaggninst = if header.flags.use_huffman {
        h_ctx
            .number_of_symbol_instances_table
            .decode(&mut h_ctx.reader)?
    } else {
        a_ctx
            .number_of_symbol_instances_decoder
            .decode(&mut a_ctx.decoder)
    }
    .ok_or(DecodeError::Symbol(SymbolError::UnexpectedOob))?;

    if refaggninst == 1 {
        decode_single_refinement_symbol(
            header,
            a_ctx,
            h_ctx,
            input_symbols,
            new_symbols,
            symwidth,
            hcheight,
            standard_tables,
        )
    } else {
        // 6.5.8.2 step 2: "If REFAGGNINST is greater than one, then decode the bitmap
        // itself using a text region decoding procedure as described in 6.4. Set the
        // parameters to this decoding procedure as shown in Table 17."
        decode_aggregation_bitmap(
            header,
            a_ctx,
            h_ctx,
            input_symbols,
            new_symbols,
            symwidth,
            hcheight,
            refaggninst as u32,
            standard_tables,
        )
    }
}

/// Decode a bitmap when REFAGGNINST = 1 (6.5.8.2.2).
#[allow(clippy::too_many_arguments)]
fn decode_single_refinement_symbol(
    header: &SymbolDictionaryHeader,
    a_ctx: &mut ArithmeticContext<'_>,
    h_ctx: &mut HuffmanContext<'_>,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    standard_tables: &StandardHuffmanTables,
) -> Result<DecodedRegion> {
    let use_huffman = header.flags.use_huffman;
    let num_input_symbols = input_symbols.len() as u32;

    // 6.5.8.2.3 Setting SBSYMCODES and SBSYMCODELEN.
    let total_symbols = num_input_symbols + header.num_new_symbols;
    let mut sbsymcodelen = 32 - (total_symbols - 1).leading_zeros();

    let (id_i, rdx_i, rdy_i) = if use_huffman {
        // See 6.5.8.2.3, the value should be at least 1 if we use huffman coding.
        sbsymcodelen = sbsymcodelen.max(1);

        let id_i = h_ctx
            .reader
            .read_bits(sbsymcodelen as u8)
            .ok_or(ParseError::UnexpectedEof)? as usize;

        let rdx_i = standard_tables
            .table_o()
            .decode(&mut h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)?;

        let rdy_i = standard_tables
            .table_o()
            .decode(&mut h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)?;

        (id_i, rdx_i, rdy_i)
    } else {
        // Use TextRegionContexts for IAID, IARDX, IARDY so they're shared with
        // REFAGGNINST > 1 cases (per spec, contexts should be reused).
        let contexts = a_ctx
            .text_region_contexts
            .get_or_insert_with(|| TextRegionContexts::new(sbsymcodelen));

        let id_i = contexts.iaid.decode(&mut a_ctx.decoder) as usize;

        let rdx_i = contexts
            .iardx
            .decode(&mut a_ctx.decoder)
            .ok_or(SymbolError::OutOfRange)?;

        let rdy_i = contexts
            .iardy
            .decode(&mut a_ctx.decoder)
            .ok_or(SymbolError::OutOfRange)?;

        (id_i, rdx_i, rdy_i)
    };

    let reference_region = if id_i < num_input_symbols as usize {
        input_symbols[id_i]
    } else {
        let new_idx = id_i - num_input_symbols as usize;
        new_symbols.get(new_idx).ok_or(SymbolError::OutOfRange)?
    };
    let mut region = DecodedRegion::new(symwidth, hcheight);

    if use_huffman {
        let bmsize = standard_tables
            .table_a()
            .decode(&mut h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)? as usize;
        h_ctx.reader.align();

        let bitmap_data = h_ctx
            .reader
            .read_bytes(bmsize)
            .ok_or(ParseError::UnexpectedEof)?;

        let mut bitmap_decoder = ArithmeticDecoder::new(bitmap_data);
        // Not sure if this is mentioned somewhere explicitly, but it seems like we
        // need to create fresh contexts for each bitmap, unlike arithmetic decoding
        // where we reuse them across multiple runs.
        let gr_template = header.flags.refinement_template;
        let num_gr_contexts = 1 << gr_template.context_bits();
        let mut gr_contexts = vec![Context::default(); num_gr_contexts];

        decode_refinement_bitmap(
            &mut bitmap_decoder,
            &mut gr_contexts,
            &mut region,
            reference_region,
            rdx_i,
            rdy_i,
            header.flags.refinement_template,
            &header.refinement_at_pixels,
            false,
        )?;
    } else {
        decode_refinement_bitmap(
            &mut a_ctx.decoder,
            &mut a_ctx.refinement_contexts,
            &mut region,
            reference_region,
            rdx_i,
            rdy_i,
            header.flags.refinement_template,
            &header.refinement_at_pixels,
            false,
        )?;
    }

    Ok(region)
}

/// Decode a bitmap when REFAGGNINST > 1 (6.5.8.2, Table 17).
///
/// Uses the text region decoding procedure (6.4) with Table 17 parameters.
#[allow(clippy::too_many_arguments)]
fn decode_aggregation_bitmap(
    header: &SymbolDictionaryHeader,
    a_ctx: &mut ArithmeticContext<'_>,
    _h_ctx: &mut HuffmanContext<'_>,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    refaggninst: u32,
    _standard_tables: &StandardHuffmanTables,
) -> Result<DecodedRegion> {
    let use_huffman = header.flags.use_huffman;
    let num_input_symbols = input_symbols.len() as u32;

    // 6.5.8.2.4 Setting SBSYMS
    // "Set SBSYMS to an array of SDNUMINSYMS + NSYMSDECODED symbols, formed by
    // concatenating the array SDINSYMS and the first NSYMSDECODED entries of
    // the array SDNEWSYMS."
    let mut sbsyms: Vec<&DecodedRegion> =
        Vec::with_capacity(input_symbols.len() + new_symbols.len());
    sbsyms.extend(input_symbols);
    for sym in new_symbols {
        sbsyms.push(sym);
    }
    // 6.5.8.2.3 Setting SBSYMCODES and SBSYMCODELEN.
    let total_symbols = num_input_symbols + header.num_new_symbols;
    let sbsymcodelen = 32 - (total_symbols - 1).leading_zeros();

    // Table 17 – Parameters used to decode a symbol's bitmap using refinement/aggregate decoding.
    let params = TextRegionParams {
        sbw: symwidth,
        sbh: hcheight,
        sbnuminstances: refaggninst,
        sbstrips: 1,
        sbdefpixel: false,
        sbcombop: CombinationOperator::Or,
        transposed: false,
        refcorner: ReferenceCorner::TopLeft,
        sbdsoffset: 0,
        sbrtemplate: header.flags.refinement_template,
        refinement_at_pixels: &header.refinement_at_pixels,
    };

    if use_huffman {
        // REFAGGNINST > 1 with Huffman is not yet supported.
        // Table 17 specifies SBHUFF = 0 (arithmetic), but the data embedding
        // for Huffman symbol dictionaries is complex and not yet implemented.
        bail!(DecodeError::Unsupported);
    }

    // For arithmetic mode, use the text region decoding with refinement.
    // Initialize text region contexts lazily if needed.
    let contexts = a_ctx
        .text_region_contexts
        .get_or_insert_with(|| TextRegionContexts::new(sbsymcodelen));

    // Use shared refinement contexts from ArithmeticContext.
    decode_text_region_refine_with_contexts(
        &mut a_ctx.decoder,
        &sbsyms,
        &params,
        contexts,
        &mut a_ctx.refinement_contexts,
    )
}
struct ArithmeticContext<'a> {
    decoder: ArithmeticDecoder<'a>,
    delta_height_decoder: IntegerDecoder,
    delta_width_decoder: IntegerDecoder,
    symbol_run_length_decoder: IntegerDecoder,
    number_of_symbol_instances_decoder: IntegerDecoder,
    bitmap_decode_contexts: Vec<Context>,
    refinement_contexts: Vec<Context>,
    // Text region contexts for REFAGGNINST (initialized lazily).
    // Contains IAID, IARDX, IARDY, and other decoders for text region decoding.
    text_region_contexts: Option<TextRegionContexts>,
}

impl<'a> ArithmeticContext<'a> {
    fn new(data: &'a [u8], header: &SymbolDictionaryHeader) -> Self {
        let decoder = ArithmeticDecoder::new(data);
        let delta_height_decoder = IntegerDecoder::new();
        let delta_width_decoder = IntegerDecoder::new();
        let symbol_run_length_decoder = IntegerDecoder::new();
        let number_of_symbol_instances_decoder = IntegerDecoder::new();

        let template = header.flags.template;
        let num_contexts = 1 << template.context_bits();
        let bitmap_decode_contexts = vec![Context::default(); num_contexts];

        let refinement_template = header.flags.refinement_template;
        let num_refinement_contexts = 1 << refinement_template.context_bits();
        let refinement_contexts = vec![Context::default(); num_refinement_contexts];

        Self {
            decoder,
            delta_height_decoder,
            delta_width_decoder,
            symbol_run_length_decoder,
            number_of_symbol_instances_decoder,
            bitmap_decode_contexts,
            refinement_contexts,
            text_region_contexts: None,
        }
    }
}

struct HuffmanContext<'a> {
    delta_height_table: &'a HuffmanTable,
    delta_width_table: &'a HuffmanTable,
    bitmap_size_table: &'a HuffmanTable,
    number_of_symbol_instances_table: &'a HuffmanTable,
    symbol_run_length_table: &'a HuffmanTable,
    reader: Reader<'a>,
}

impl<'a> HuffmanContext<'a> {
    fn new(
        data: &'a [u8],
        header: &SymbolDictionaryHeader,
        referred_tables: &'a [HuffmanTable],
        standard_tables: &'a StandardHuffmanTables,
    ) -> Result<Self> {
        let reader = Reader::new(data);

        let custom_count = [
            header.flags.delta_height_table == HuffmanTableSelection::UserSupplied,
            header.flags.delta_width_table == HuffmanTableSelection::UserSupplied,
            header.flags.bitmap_size_table == HuffmanTableSelection::UserSupplied,
            header.flags.aggregate_instance_table == HuffmanTableSelection::UserSupplied,
        ]
        .into_iter()
        .filter(|x| *x)
        .count();

        if referred_tables.len() < custom_count {
            bail!(HuffmanError::MissingTables);
        }

        let mut custom_idx = 0;
        let mut get_custom = || -> &HuffmanTable {
            let table = &referred_tables[custom_idx];
            custom_idx += 1;
            table
        };

        // Select Huffman tables based on flags (7.4.2.1.6).
        let mut get_table = |selection: HuffmanTableSelection| -> &HuffmanTable {
            match selection {
                HuffmanTableSelection::TableB1 => standard_tables.table_a(),
                HuffmanTableSelection::TableB2 => standard_tables.table_b(),
                HuffmanTableSelection::TableB3 => standard_tables.table_c(),
                HuffmanTableSelection::TableB4 => standard_tables.table_d(),
                HuffmanTableSelection::TableB5 => standard_tables.table_e(),
                HuffmanTableSelection::UserSupplied => get_custom(),
            }
        };

        let delta_height_table = get_table(header.flags.delta_height_table);
        let delta_width_table = get_table(header.flags.delta_width_table);
        let bitmap_size_table = get_table(header.flags.bitmap_size_table);
        let number_of_symbol_instances_table = get_table(header.flags.aggregate_instance_table);
        let symbol_run_length_table = get_table(HuffmanTableSelection::TableB1);

        Ok(Self {
            reader,
            delta_height_table,
            delta_width_table,
            bitmap_size_table,
            number_of_symbol_instances_table,
            symbol_run_length_table,
        })
    }
}

/// Huffman table selection for symbol dictionary fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HuffmanTableSelection {
    TableB1,
    TableB2,
    TableB3,
    TableB4,
    TableB5,
    UserSupplied,
}

/// Parsed symbol dictionary segment flags (7.4.2.1.1).
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionaryFlags {
    pub(crate) use_huffman: bool,
    pub(crate) use_refagg: bool,
    pub(crate) delta_height_table: HuffmanTableSelection,
    pub(crate) delta_width_table: HuffmanTableSelection,
    pub(crate) bitmap_size_table: HuffmanTableSelection,
    pub(crate) aggregate_instance_table: HuffmanTableSelection,
    pub(crate) _bitmap_context_used: bool,
    pub(crate) _bitmap_context_retained: bool,
    pub(crate) template: Template,
    pub(crate) refinement_template: RefinementTemplate,
}

/// Parsed symbol dictionary segment header (7.4.2.1).
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionaryHeader {
    pub(crate) flags: SymbolDictionaryFlags,
    pub(crate) adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) refinement_at_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) num_exported_symbols: u32,
    pub(crate) num_new_symbols: u32,
}

/// Parse a symbol dictionary segment header (7.4.2.1).
fn parse(reader: &mut Reader<'_>) -> Result<SymbolDictionaryHeader> {
    let flags_word = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let use_huffman = flags_word & 0x0001 != 0;
    let use_refagg = flags_word & 0x0002 != 0;

    let delta_height_table = match (flags_word >> 2) & 0x03 {
        0 => HuffmanTableSelection::TableB4,
        1 => HuffmanTableSelection::TableB5,
        3 => HuffmanTableSelection::UserSupplied,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let delta_width_table = match (flags_word >> 4) & 0x03 {
        0 => HuffmanTableSelection::TableB2,
        1 => HuffmanTableSelection::TableB3,
        3 => HuffmanTableSelection::UserSupplied,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let bitmap_size_table = if flags_word & 0x0040 != 0 {
        HuffmanTableSelection::UserSupplied
    } else {
        HuffmanTableSelection::TableB1
    };

    let aggregate_instance_table = if flags_word & 0x0080 != 0 {
        HuffmanTableSelection::UserSupplied
    } else {
        HuffmanTableSelection::TableB1
    };

    let bitmap_context_used = flags_word & 0x0100 != 0;
    let bitmap_context_retained = flags_word & 0x0200 != 0;
    let template = Template::from_byte((flags_word >> 10) as u8);
    let refinement_template = RefinementTemplate::from_byte((flags_word >> 12) as u8);

    let flags = SymbolDictionaryFlags {
        use_huffman,
        use_refagg,
        delta_height_table,
        delta_width_table,
        bitmap_size_table,
        aggregate_instance_table,
        // TODO: Implement those.
        _bitmap_context_used: bitmap_context_used,
        _bitmap_context_retained: bitmap_context_retained,
        template,
        refinement_template,
    };

    let at_pixels = if !use_huffman {
        parse_adaptive_template_pixels(reader, template, false)?
    } else {
        Vec::new()
    };

    let refinement_at_pixels = if use_refagg && refinement_template == RefinementTemplate::Template0
    {
        parse_refinement_at_pixels(reader)?
    } else {
        Vec::new()
    };
    let num_exported_symbols = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let num_new_symbols = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    Ok(SymbolDictionaryHeader {
        flags,
        adaptive_template_pixels: at_pixels,
        refinement_at_pixels,
        num_exported_symbols,
        num_new_symbols,
    })
}

/// A decoded symbol dictionary segment.
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionary {
    pub(crate) exported_symbols: Vec<DecodedRegion>,
}

/// Decode a height class collective bitmap (6.5.9).
///
/// "This field is only present if SDHUFF = 1 and SDREFAGG = 0." (6.5.9)
#[allow(clippy::too_many_arguments)]
fn decode_height_class_collective_bitmap(
    reader: &mut Reader<'_>,
    sdhuffbmsize: &HuffmanTable,
    new_symbols: &mut Vec<DecodedRegion>,
    new_sym_widths: &[u32],
    hcfirstsym: u32,
    nsymsdecoded: u32,
    totwidth: u32,
    hcheight: u32,
) -> Result<()> {
    // "1) Read the size in bytes using the SDHUFFBMSIZE Huffman table.
    // Let BMSIZE be the value decoded."
    let bmsize = sdhuffbmsize
        .decode(reader)?
        .ok_or(HuffmanError::UnexpectedOob)? as u32;

    // "2) Skip over any bits remaining in the last byte read."
    reader.align();

    // Decode the collective bitmap
    let collective_bitmap = if bmsize == 0 {
        // "3) If BMSIZE is zero, then the bitmap is stored uncompressed, and the
        // actual size in bytes is: HCHEIGHT × ⌈TOTWIDTH / 8⌉"
        let row_bytes = totwidth.div_ceil(8);

        let mut bitmap = DecodedRegion::new(totwidth, hcheight);
        for y in 0..hcheight {
            for byte_x in 0..row_bytes {
                let byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
                for bit in 0..8 {
                    let x = byte_x * 8 + bit;
                    if x < totwidth {
                        let pixel = (byte >> (7 - bit)) & 1 != 0;
                        bitmap.set_pixel(x, y, pixel);
                    }
                }
            }
        }
        bitmap
    } else {
        // "4) Otherwise, decode the bitmap using a generic bitmap decoding procedure
        // as described in 6.2. Set the parameters to this decoding procedure as
        // shown in Table 19." (MMR = 1)
        let bitmap_data = reader
            .read_bytes(bmsize as usize)
            .ok_or(ParseError::UnexpectedEof)?;

        let mut bitmap = DecodedRegion::new(totwidth, hcheight);
        decode_bitmap_mmr(&mut bitmap, bitmap_data)?;
        bitmap
    };

    // "Break up the bitmap B_HC as follows to obtain the symbols
    // SDNEWSYMS[HCFIRSTSYM] through SDNEWSYMS[NSYMSDECODED − 1]." (6.5.5, step 4d)
    //
    // "B_HC contains the NSYMSDECODED − HCFIRSTSYM symbols concatenated left-to-right,
    // with no intervening gaps."
    let mut x_offset: u32 = 0;
    for i in hcfirstsym..nsymsdecoded {
        let sym_width = new_sym_widths[i as usize];
        let mut symbol = DecodedRegion::new(sym_width, hcheight);

        // Copy pixels from collective bitmap to individual symbol
        for y in 0..hcheight {
            for x in 0..sym_width {
                let pixel = collective_bitmap.get_pixel(x_offset + x, y);
                symbol.set_pixel(x, y, pixel);
            }
        }

        new_symbols.push(symbol);
        x_offset += sym_width;
    }

    Ok(())
}

/// Determine exported symbols (6.5.10).
///
/// "The symbols that may be exported from a given dictionary include any of the
/// symbols that are input to the dictionary, plus any of the symbols defined in
/// the dictionary." (6.5.10)
///
/// The `decode_value` closure decodes the run length value:
/// - For Huffman coding (SDHUFF=1): uses Table B.1
/// - For arithmetic coding (SDHUFF=0): uses the IAEX integer decoder
fn decode_exported_symbols_with<F>(
    num_input_symbols: u32,
    num_exported: u32,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    mut decode_value: F,
) -> Result<Vec<DecodedRegion>>
where
    F: FnMut() -> Result<Option<i32>>,
{
    let num_new_symbols = new_symbols.len() as u32;
    let total_symbols = num_input_symbols + num_new_symbols;

    // "1) Set: EXINDEX = 0, CUREXFLAG = 0"
    let mut exindex: u32 = 0;
    let mut curexflag: bool = false;

    // EXFLAGS array - one bit per symbol indicating if exported
    let mut exflags = vec![false; total_symbols as usize];

    // "5) Repeat steps 2) through 4) until EXINDEX = SDNUMINSYMS + SDNUMNEWSYMS"
    while exindex < total_symbols {
        // "2) Decode a value using Table B.1 if SDHUFF is 1, or the IAEX integer
        // arithmetic decoding procedure if SDHUFF is 0. Let EXRUNLENGTH be the
        // decoded value."
        let exrunlength =
            decode_value()?.ok_or(DecodeError::Huffman(HuffmanError::UnexpectedOob))?;

        if exrunlength < 0 {
            bail!(HuffmanError::InvalidCode);
        }

        let exrunlength = exrunlength as u32;

        // "3) Set EXFLAGS[EXINDEX] through EXFLAGS[EXINDEX + EXRUNLENGTH - 1]
        // to CUREXFLAG."
        for i in 0..exrunlength {
            let idx = (exindex + i) as usize;
            if idx < exflags.len() {
                exflags[idx] = curexflag;
            }
        }

        // "4) Set: EXINDEX = EXINDEX + EXRUNLENGTH, CUREXFLAG = NOT(CUREXFLAG)"
        exindex += exrunlength;
        curexflag = !curexflag;
    }

    // "8) For each value of I from 0 to SDNUMINSYMS + SDNUMNEWSYMS - 1, if
    // EXFLAGS[I] = 1 then perform the following steps:"
    let mut exported = Vec::with_capacity(num_exported as usize);

    for (i, &is_exported) in exflags.iter().enumerate() {
        if is_exported {
            let symbol = if (i as u32) < num_input_symbols {
                // "a) If I < SDNUMINSYMS then set: SDEXSYMS[J] = SDINSYMS[I]"
                input_symbols[i].clone()
            } else {
                // "b) If I >= SDNUMINSYMS then set:
                // SDEXSYMS[J] = SDNEWSYMS[I - SDNUMINSYMS]"
                let new_idx = i - num_input_symbols as usize;
                new_symbols[new_idx].clone()
            };
            exported.push(symbol);
        }
    }

    if exported.len() != num_exported as usize {
        bail!(SymbolError::NoSymbols);
    }

    Ok(exported)
}
