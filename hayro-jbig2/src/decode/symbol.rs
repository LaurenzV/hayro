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
    let num_new_symbols = header.num_new_symbols;

    let mut ctx = SymbolDecodeContext {
        a_ctx: ArithmeticContext::new(data, &header),
        h_ctx: HuffmanContext::new(data, &header, referred_tables, standard_tables)?,
        num_input_symbols: input_symbols.len() as u32,
        new_symbols: Vec::with_capacity(num_new_symbols as usize),
        header,
        input_symbols,
        standard_tables,
    };

    let read_height_class_delta = |ctx: &mut SymbolDecodeContext<'_>| {
        if ctx.header.flags.use_huffman {
            ctx.h_ctx
                .height_class_delta_table
                .decode(&mut ctx.h_ctx.reader)
        } else {
            Ok(ctx
                .a_ctx
                .height_class_delta_decoder
                .decode(&mut ctx.a_ctx.decoder))
        }
    };

    let read_symbol_width_delta = |ctx: &mut SymbolDecodeContext<'_>| {
        if ctx.header.flags.use_huffman {
            ctx.h_ctx
                .symbol_width_delta_table
                .decode(&mut ctx.h_ctx.reader)
        } else {
            Ok(ctx
                .a_ctx
                .symbol_width_delta_decoder
                .decode(&mut ctx.a_ctx.decoder))
        }
    };
    // Only used if SDHUFF = 1 and SDREFAGG = 0.
    let mut symbol_widths = Vec::with_capacity(num_new_symbols as usize);

    let mut height_class_height: u32 = 0;
    let mut symbols_decoded_count: u32 = 0;

    while symbols_decoded_count < num_new_symbols {
        let height_class_delta =
            read_height_class_delta(&mut ctx)?.ok_or(SymbolError::OutOfRange)?;

        height_class_height = height_class_height
            .checked_add_signed(height_class_delta)
            .ok_or(RegionError::InvalidDimension)?;

        let mut symbol_width: u32 = 0;
        let mut total_width: u32 = 0;
        let height_class_first_symbol = symbols_decoded_count;

        // "If the result of this decoding is OOB then all the symbols
        // in this height class have been decoded."
        while let Some(width_delta) = read_symbol_width_delta(&mut ctx)? {
            // Prevent infinite loop for invalid files.
            if symbols_decoded_count >= num_new_symbols {
                bail!(SymbolError::TooManySymbols)
            }

            symbol_width = symbol_width
                .checked_add_signed(width_delta)
                .ok_or(RegionError::InvalidDimension)?;
            total_width = total_width
                .checked_add(symbol_width)
                .ok_or(RegionError::InvalidDimension)?;

            match (ctx.header.flags.use_huffman, ctx.header.flags.use_refagg) {
                (false, false) => {
                    // Decode a single symbol using a simple generic decoding procedure,
                    // described in 6.5.8.1.
                    let mut region = DecodedRegion::new(symbol_width, height_class_height);
                    generic::decode_bitmap_arithmetic_coding(
                        &mut region,
                        &mut ctx.a_ctx.decoder,
                        &mut ctx.a_ctx.generic_region_contexts,
                        ctx.header.flags.template,
                        false,
                        &ctx.header.adaptive_template_pixels,
                    )?;

                    ctx.new_symbols.push(region);
                }
                (true, false) => {
                    // Decode a single symbol width. We don't actually decode the symbols
                    // yet, those will be decoded later on from the collective bitmap.
                    symbol_widths.push(symbol_width);
                }
                (_, true) => {
                    // Also decode a single symbol, but using refinement-aggregation.
                    // In this case, we can have both, huffman and arithmetic coding.
                    let symbol = decode_bitmap_refagg(&mut ctx, symbol_width, height_class_height)?;

                    ctx.new_symbols.push(symbol);
                }
            }

            symbols_decoded_count += 1;
        }

        if ctx.header.flags.use_huffman && !ctx.header.flags.use_refagg {
            // Now, we use the symbol widths to decode the collective bitmap.
            decode_height_class_collective_bitmap(
                &mut ctx.h_ctx.reader,
                ctx.h_ctx.collective_bitmap_size_table,
                &mut ctx.new_symbols,
                &symbol_widths,
                height_class_first_symbol,
                symbols_decoded_count,
                total_width,
                height_class_height,
            )?;
        }
    }

    let exported = export_symbols(&mut ctx)?;

    Ok(SymbolDictionary {
        exported_symbols: exported,
    })
}

/// A decoded symbol dictionary segment.
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionary {
    pub(crate) exported_symbols: Vec<DecodedRegion>,
}

/// Decode a symbol bitmap using refinement/aggregate coding (6.5.8.2).
fn decode_bitmap_refagg(
    ctx: &mut SymbolDecodeContext<'_>,
    symbol_width: u32,
    height_class_height: u32,
) -> Result<DecodedRegion> {
    // 6.5.8.2.1 Number of symbol instances in aggregation.
    let aggregation_instance_count = if ctx.header.flags.use_huffman {
        ctx.h_ctx
            .aggregation_instance_count_table
            .decode(&mut ctx.h_ctx.reader)?
    } else {
        ctx.a_ctx
            .aggregation_instance_count_decoder
            .decode(&mut ctx.a_ctx.decoder)
    }
    .ok_or(DecodeError::Symbol(SymbolError::UnexpectedOob))?;

    if aggregation_instance_count == 1 {
        decode_single_refinement_symbol(ctx, symbol_width, height_class_height)
    } else {
        // 6.5.8.2 step 2: "If REFAGGNINST is greater than one, then decode the bitmap
        // itself using a text region decoding procedure as described in 6.4. Set the
        // parameters to this decoding procedure as shown in Table 17."
        decode_aggregation_bitmap(
            ctx,
            symbol_width,
            height_class_height,
            aggregation_instance_count as u32,
        )
    }
}

/// Decode a bitmap when REFAGGNINST = 1 (6.5.8.2.2).
fn decode_single_refinement_symbol(
    ctx: &mut SymbolDecodeContext<'_>,
    symbol_width: u32,
    height_class_height: u32,
) -> Result<DecodedRegion> {
    let use_huffman = ctx.header.flags.use_huffman;

    // 6.5.8.2.3 Setting SBSYMCODES and SBSYMCODELEN.
    let total_symbols = ctx.num_input_symbols + ctx.header.num_new_symbols;
    let mut sbsymcodelen = 32 - (total_symbols - 1).leading_zeros();

    let (id_i, rdx_i, rdy_i) = if use_huffman {
        // See 6.5.8.2.3, the value should be at least 1 if we use huffman coding.
        sbsymcodelen = sbsymcodelen.max(1);

        let id_i = ctx
            .h_ctx
            .reader
            .read_bits(sbsymcodelen as u8)
            .ok_or(ParseError::UnexpectedEof)? as usize;

        let rdx_i = ctx
            .standard_tables
            .table_o()
            .decode(&mut ctx.h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)?;

        let rdy_i = ctx
            .standard_tables
            .table_o()
            .decode(&mut ctx.h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)?;

        (id_i, rdx_i, rdy_i)
    } else {
        // Use TextRegionContexts for IAID, IARDX, IARDY so they're shared with
        // REFAGGNINST > 1 cases (per spec, contexts should be reused).
        let contexts = ctx
            .a_ctx
            .text_region_contexts
            .get_or_insert_with(|| TextRegionContexts::new(sbsymcodelen));

        let id_i = contexts.iaid.decode(&mut ctx.a_ctx.decoder) as usize;

        let rdx_i = contexts
            .iardx
            .decode(&mut ctx.a_ctx.decoder)
            .ok_or(SymbolError::UnexpectedOob)?;

        let rdy_i = contexts
            .iardy
            .decode(&mut ctx.a_ctx.decoder)
            .ok_or(SymbolError::UnexpectedOob)?;

        (id_i, rdx_i, rdy_i)
    };

    let reference_region = if id_i < ctx.num_input_symbols as usize {
        ctx.input_symbols[id_i]
    } else {
        let new_idx = id_i - ctx.num_input_symbols as usize;
        ctx.new_symbols
            .get(new_idx)
            .ok_or(SymbolError::OutOfRange)?
    };
    let mut region = DecodedRegion::new(symbol_width, height_class_height);

    if use_huffman {
        let bmsize = ctx
            .standard_tables
            .table_a()
            .decode(&mut ctx.h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)? as usize;
        ctx.h_ctx.reader.align();

        let bitmap_data = ctx
            .h_ctx
            .reader
            .read_bytes(bmsize)
            .ok_or(ParseError::UnexpectedEof)?;

        let mut bitmap_decoder = ArithmeticDecoder::new(bitmap_data);
        // Not sure if this is mentioned somewhere explicitly, but it seems like we
        // need to create fresh contexts for each bitmap, unlike arithmetic decoding
        // where we reuse them across multiple runs.
        let gr_template = ctx.header.flags.refinement_template;
        let num_gr_contexts = 1 << gr_template.context_bits();
        let mut gr_contexts = vec![Context::default(); num_gr_contexts];

        decode_refinement_bitmap(
            &mut bitmap_decoder,
            &mut gr_contexts,
            &mut region,
            reference_region,
            rdx_i,
            rdy_i,
            ctx.header.flags.refinement_template,
            &ctx.header.refinement_at_pixels,
            false,
        )?;
    } else {
        decode_refinement_bitmap(
            &mut ctx.a_ctx.decoder,
            &mut ctx.a_ctx.refinement_region_contexts,
            &mut region,
            reference_region,
            rdx_i,
            rdy_i,
            ctx.header.flags.refinement_template,
            &ctx.header.refinement_at_pixels,
            false,
        )?;
    }

    Ok(region)
}

/// Decode a bitmap when REFAGGNINST > 1 (6.5.8.2, Table 17).
///
/// Uses the text region decoding procedure (6.4) with Table 17 parameters.
fn decode_aggregation_bitmap(
    ctx: &mut SymbolDecodeContext<'_>,
    symbol_width: u32,
    height_class_height: u32,
    aggregation_instance_count: u32,
) -> Result<DecodedRegion> {
    let use_huffman = ctx.header.flags.use_huffman;

    // 6.5.8.2.4 Setting SBSYMS
    // "Set SBSYMS to an array of SDNUMINSYMS + NSYMSDECODED symbols, formed by
    // concatenating the array SDINSYMS and the first NSYMSDECODED entries of
    // the array SDNEWSYMS."
    let mut sbsyms: Vec<&DecodedRegion> =
        Vec::with_capacity(ctx.input_symbols.len() + ctx.new_symbols.len());
    sbsyms.extend(ctx.input_symbols.iter().copied());
    for sym in &ctx.new_symbols {
        sbsyms.push(sym);
    }
    // 6.5.8.2.3 Setting SBSYMCODES and SBSYMCODELEN.
    let total_symbols = ctx.num_input_symbols + ctx.header.num_new_symbols;
    let sbsymcodelen = 32 - (total_symbols - 1).leading_zeros();

    // Table 17 – Parameters used to decode a symbol's bitmap using refinement/aggregate decoding.
    let params = TextRegionParams {
        sbw: symbol_width,
        sbh: height_class_height,
        sbnuminstances: aggregation_instance_count,
        sbstrips: 1,
        sbdefpixel: false,
        sbcombop: CombinationOperator::Or,
        transposed: false,
        refcorner: ReferenceCorner::TopLeft,
        sbdsoffset: 0,
        sbrtemplate: ctx.header.flags.refinement_template,
        refinement_at_pixels: &ctx.header.refinement_at_pixels,
    };

    if use_huffman {
        // REFAGGNINST > 1 with Huffman is not yet supported.
        // Table 17 specifies SBHUFF = 0 (arithmetic), but the data embedding
        // for Huffman symbol dictionaries is complex and not yet implemented.
        bail!(DecodeError::Unsupported);
    }

    // For arithmetic mode, use the text region decoding with refinement.
    // Initialize text region contexts lazily if needed.
    let contexts = ctx
        .a_ctx
        .text_region_contexts
        .get_or_insert_with(|| TextRegionContexts::new(sbsymcodelen));

    // Use shared refinement contexts from ArithmeticContext.
    decode_text_region_refine_with_contexts(
        &mut ctx.a_ctx.decoder,
        &sbsyms,
        &params,
        contexts,
        &mut ctx.a_ctx.refinement_region_contexts,
    )
}

struct SymbolDecodeContext<'a> {
    header: SymbolDictionaryHeader,
    a_ctx: ArithmeticContext<'a>,
    h_ctx: HuffmanContext<'a>,
    input_symbols: &'a [&'a DecodedRegion],
    num_input_symbols: u32,
    standard_tables: &'a StandardHuffmanTables,
    new_symbols: Vec<DecodedRegion>,
}

struct ArithmeticContext<'a> {
    decoder: ArithmeticDecoder<'a>,
    /// `IADH`
    height_class_delta_decoder: IntegerDecoder,
    /// `IADW`
    symbol_width_delta_decoder: IntegerDecoder,
    /// `IAEX`
    export_run_length_decoder: IntegerDecoder,
    /// `IAAI`
    aggregation_instance_count_decoder: IntegerDecoder,
    generic_region_contexts: Vec<Context>,
    refinement_region_contexts: Vec<Context>,
    /// `IAID`, `IARDX`, `IARDY`, etc.
    text_region_contexts: Option<TextRegionContexts>,
}

impl<'a> ArithmeticContext<'a> {
    fn new(data: &'a [u8], header: &SymbolDictionaryHeader) -> Self {
        let decoder = ArithmeticDecoder::new(data);
        let height_class_delta_decoder = IntegerDecoder::new();
        let symbol_width_delta_decoder = IntegerDecoder::new();
        let export_run_length_decoder = IntegerDecoder::new();
        let aggregation_instance_count_decoder = IntegerDecoder::new();

        let template = header.flags.template;
        let num_contexts = 1 << template.context_bits();
        let generic_region_contexts = vec![Context::default(); num_contexts];

        let refinement_template = header.flags.refinement_template;
        let num_refinement_contexts = 1 << refinement_template.context_bits();
        let refinement_region_contexts = vec![Context::default(); num_refinement_contexts];

        Self {
            decoder,
            height_class_delta_decoder,
            symbol_width_delta_decoder,
            export_run_length_decoder,
            aggregation_instance_count_decoder,
            generic_region_contexts,
            refinement_region_contexts,
            text_region_contexts: None,
        }
    }
}

struct HuffmanContext<'a> {
    /// `SDHUFFDH`
    height_class_delta_table: &'a HuffmanTable,
    /// `SDHUFFDW`
    symbol_width_delta_table: &'a HuffmanTable,
    /// `SDHUFFBMSIZE`
    collective_bitmap_size_table: &'a HuffmanTable,
    /// `SDHUFFAGGINST`
    aggregation_instance_count_table: &'a HuffmanTable,
    export_run_length_table: &'a HuffmanTable,
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
            header.flags.collective_bitmap_size_table == HuffmanTableSelection::UserSupplied,
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

        let height_class_delta_table = get_table(header.flags.delta_height_table);
        let symbol_width_delta_table = get_table(header.flags.delta_width_table);
        let collective_bitmap_size_table = get_table(header.flags.collective_bitmap_size_table);
        let aggregation_instance_count_table = get_table(header.flags.aggregate_instance_table);
        let export_run_length_table = get_table(HuffmanTableSelection::TableB1);

        Ok(Self {
            reader,
            height_class_delta_table,
            symbol_width_delta_table,
            collective_bitmap_size_table,
            aggregation_instance_count_table,
            export_run_length_table,
        })
    }
}

/// Decode a height class collective bitmap (6.5.9).
///
/// "This field is only present if SDHUFF = 1 and SDREFAGG = 0." (6.5.9)
#[allow(clippy::too_many_arguments)]
fn decode_height_class_collective_bitmap(
    reader: &mut Reader<'_>,
    collective_bitmap_size_table: &HuffmanTable,
    new_symbols: &mut Vec<DecodedRegion>,
    new_symbols_widths: &[u32],
    height_class_first_symbol: u32,
    symbols_decoded_count: u32,
    total_width: u32,
    height_class_height: u32,
) -> Result<()> {
    // "1) Read the size in bytes using the SDHUFFBMSIZE Huffman table.
    // Let BMSIZE be the value decoded."
    let bmsize = collective_bitmap_size_table
        .decode(reader)?
        .ok_or(HuffmanError::UnexpectedOob)? as u32;

    // "2) Skip over any bits remaining in the last byte read."
    reader.align();

    // Decode the collective bitmap
    let collective_bitmap = if bmsize == 0 {
        // "3) If BMSIZE is zero, then the bitmap is stored uncompressed, and the
        // actual size in bytes is: HCHEIGHT × ⌈TOTWIDTH / 8⌉"
        let row_bytes = total_width.div_ceil(8);

        let mut bitmap = DecodedRegion::new(total_width, height_class_height);
        for y in 0..height_class_height {
            for byte_x in 0..row_bytes {
                let byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
                for bit in 0..8 {
                    let x = byte_x * 8 + bit;
                    if x < total_width {
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

        let mut bitmap = DecodedRegion::new(total_width, height_class_height);
        decode_bitmap_mmr(&mut bitmap, bitmap_data)?;
        bitmap
    };

    // "Break up the bitmap B_HC as follows to obtain the symbols
    // SDNEWSYMS[HCFIRSTSYM] through SDNEWSYMS[NSYMSDECODED − 1]." (6.5.5, step 4d)
    //
    // "B_HC contains the NSYMSDECODED − HCFIRSTSYM symbols concatenated left-to-right,
    // with no intervening gaps."
    let mut x_offset: u32 = 0;
    for i in height_class_first_symbol..symbols_decoded_count {
        let sym_width = new_symbols_widths[i as usize];
        let mut symbol = DecodedRegion::new(sym_width, height_class_height);

        // Copy pixels from collective bitmap to individual symbol
        for y in 0..height_class_height {
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

/// Exported symbols (6.5.10).
fn export_symbols(ctx: &mut SymbolDecodeContext<'_>) -> Result<Vec<DecodedRegion>> {
    let mut decode_export_run_length = || {
        if ctx.header.flags.use_huffman {
            ctx.h_ctx
                .export_run_length_table
                .decode(&mut ctx.h_ctx.reader)
        } else {
            Ok(ctx
                .a_ctx
                .export_run_length_decoder
                .decode(&mut ctx.a_ctx.decoder))
        }
    };

    let num_new_symbols = ctx.new_symbols.len() as u32;
    let total_symbols = ctx.num_input_symbols + num_new_symbols;

    // "1) Set: EXINDEX = 0, CUREXFLAG = 0"
    let mut export_index: u32 = 0;
    let mut current_export_flag: bool = false;

    // EXFLAGS array - one bit per symbol indicating if exported
    let mut export_flags = vec![false; total_symbols as usize];

    // "5) Repeat steps 2) through 4) until EXINDEX = SDNUMINSYMS + SDNUMNEWSYMS"
    while export_index < total_symbols {
        // "2) Decode a value using Table B.1 if SDHUFF is 1, or the IAEX integer
        // arithmetic decoding procedure if SDHUFF is 0. Let EXRUNLENGTH be the
        // decoded value."
        let export_run_length =
            decode_export_run_length()?.ok_or(DecodeError::Huffman(HuffmanError::UnexpectedOob))?;

        if export_run_length < 0 {
            bail!(HuffmanError::InvalidCode);
        }

        let export_run_length = export_run_length as u32;

        // "3) Set EXFLAGS[EXINDEX] through EXFLAGS[EXINDEX + EXRUNLENGTH - 1]
        // to CUREXFLAG."
        for i in 0..export_run_length {
            let idx = (export_index + i) as usize;
            if idx < export_flags.len() {
                export_flags[idx] = current_export_flag;
            }
        }

        // "4) Set: EXINDEX = EXINDEX + EXRUNLENGTH, CUREXFLAG = NOT(CUREXFLAG)"
        export_index += export_run_length;
        current_export_flag = !current_export_flag;
    }

    // "8) For each value of I from 0 to SDNUMINSYMS + SDNUMNEWSYMS - 1, if
    // EXFLAGS[I] = 1 then perform the following steps:"
    let mut exported = Vec::with_capacity(ctx.header.num_exported_symbols as usize);

    for (i, &is_exported) in export_flags.iter().enumerate() {
        if is_exported {
            let symbol = if (i as u32) < ctx.num_input_symbols {
                // "a) If I < SDNUMINSYMS then set: SDEXSYMS[J] = SDINSYMS[I]"
                ctx.input_symbols[i].clone()
            } else {
                // "b) If I >= SDNUMINSYMS then set:
                // SDEXSYMS[J] = SDNEWSYMS[I - SDNUMINSYMS]"
                let new_idx = i - ctx.num_input_symbols as usize;
                ctx.new_symbols[new_idx].clone()
            };
            exported.push(symbol);
        }
    }

    if exported.len() != ctx.header.num_exported_symbols as usize {
        bail!(SymbolError::NoSymbols);
    }

    Ok(exported)
}

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
    pub(crate) collective_bitmap_size_table: HuffmanTableSelection,
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

    let collective_bitmap_size_table = if flags_word & 0x0040 != 0 {
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
        collective_bitmap_size_table,
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
