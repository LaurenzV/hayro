//! Symbol dictionary segment parsing and decoding (7.4.2, 6.5).

use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::decode::generic::{decode_bitmap_mmr, parse_adaptive_template_pixels};
use crate::decode::text::{
    ReferenceCorner, TextRegionContexts, TextRegionParams, decode_text_region_refine_with_contexts,
};
use crate::decode::{
    AdaptiveTemplatePixel, CombinationOperator, RefinementTemplate, Template,
    parse_refinement_at_pixels,
};
use crate::decode::{generic, generic_refinement};
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
        symbol_widths: Vec::with_capacity(num_new_symbols as usize),
        height_class_first_symbol: 0,
        symbols_decoded_count: 0,
        total_width: 0,
        height_class_height: 0,
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

    while ctx.symbols_decoded_count < num_new_symbols {
        let height_class_delta =
            read_height_class_delta(&mut ctx)?.ok_or(SymbolError::OutOfRange)?;

        ctx.height_class_height = ctx
            .height_class_height
            .checked_add_signed(height_class_delta)
            .ok_or(RegionError::InvalidDimension)?;

        let mut symbol_width: u32 = 0;
        ctx.total_width = 0;
        ctx.height_class_first_symbol = ctx.symbols_decoded_count;

        // "If the result of this decoding is OOB then all the symbols
        // in this height class have been decoded."
        while let Some(width_delta) = read_symbol_width_delta(&mut ctx)? {
            // Prevent infinite loop for invalid files.
            if ctx.symbols_decoded_count >= num_new_symbols {
                bail!(SymbolError::TooManySymbols)
            }

            symbol_width = symbol_width
                .checked_add_signed(width_delta)
                .ok_or(RegionError::InvalidDimension)?;
            ctx.total_width = ctx
                .total_width
                .checked_add(symbol_width)
                .ok_or(RegionError::InvalidDimension)?;

            match (ctx.header.flags.use_huffman, ctx.header.flags.use_refagg) {
                (false, false) => {
                    // Decode a single symbol using a simple generic decoding procedure,
                    // described in 6.5.8.1.
                    let mut region = DecodedRegion::new(symbol_width, ctx.height_class_height);
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
                    ctx.symbol_widths.push(symbol_width);
                }
                (_, true) => {
                    // Also decode a single symbol, but using refinement-aggregation.
                    // In this case, we can have both, huffman and arithmetic coding.
                    let symbol = decode_refinement_aggregation_bitmap(&mut ctx, symbol_width)?;

                    ctx.new_symbols.push(symbol);
                }
            }

            ctx.symbols_decoded_count += 1;
        }

        if ctx.header.flags.use_huffman && !ctx.header.flags.use_refagg {
            // Now, we use the symbol widths to decode the collective bitmap.
            decode_collective_bitmap(&mut ctx)?;
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
fn decode_refinement_aggregation_bitmap(
    ctx: &mut SymbolDecodeContext<'_>,
    symbol_width: u32,
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
        decode_refinement_bitmap(ctx, symbol_width)
    } else {
        decode_aggregation_bitmap(ctx, symbol_width, aggregation_instance_count as u32)
    }
}

/// Decode a refinement bitmap symbol (6.5.8.2.2).
fn decode_refinement_bitmap(
    ctx: &mut SymbolDecodeContext<'_>,
    symbol_width: u32,
) -> Result<DecodedRegion> {
    let use_huffman = ctx.header.flags.use_huffman;

    let mut sbsymcodelen = 32 - (ctx.total_symbols() - 1).leading_zeros();

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
    let mut region = DecodedRegion::new(symbol_width, ctx.height_class_height);

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

        generic_refinement::decode_bitmap(
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
        generic_refinement::decode_bitmap(
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
    let sbsymcodelen = 32 - (ctx.total_symbols() - 1).leading_zeros();

    // Table 17 – Parameters used to decode a symbol's bitmap using refinement/aggregate decoding.
    let params = TextRegionParams {
        sbw: symbol_width,
        sbh: ctx.height_class_height,
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
    /// Only used if SDHUFF = 1 and SDREFAGG = 0.
    symbol_widths: Vec<u32>,
    height_class_first_symbol: u32,
    symbols_decoded_count: u32,
    total_width: u32,
    height_class_height: u32,
}

impl SymbolDecodeContext<'_> {
    fn total_symbols(&self) -> u32 {
        self.num_input_symbols + self.header.num_new_symbols
    }
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
fn decode_collective_bitmap(ctx: &mut SymbolDecodeContext<'_>) -> Result<()> {
    let bitmap_size = ctx
        .h_ctx
        .collective_bitmap_size_table
        .decode(&mut ctx.h_ctx.reader)?
        .ok_or(HuffmanError::UnexpectedOob)? as u32;

    ctx.h_ctx.reader.align();

    let collective_bitmap = if bitmap_size == 0 {
        // Bitmap is stored uncompressed, so we basically just copy-paste
        // each byte. Rows are padded to the byte boundary.
        let row_bytes = ctx.total_width.div_ceil(8);

        let mut bitmap = DecodedRegion::new(ctx.total_width, ctx.height_class_height);
        for y in 0..ctx.height_class_height {
            for byte_x in 0..row_bytes {
                let byte = ctx
                    .h_ctx
                    .reader
                    .read_byte()
                    .ok_or(ParseError::UnexpectedEof)?;
                for bit in 0..8 {
                    let x = byte_x * 8 + bit;
                    if x < ctx.total_width {
                        let pixel = (byte >> (7 - bit)) & 1 != 0;
                        bitmap.set_pixel(x, y, pixel);
                    }
                }
            }
        }
        bitmap
    } else {
        // Otherwise, we need to use MMR decoding.
        let bitmap_data = ctx
            .h_ctx
            .reader
            .read_bytes(bitmap_size as usize)
            .ok_or(ParseError::UnexpectedEof)?;

        let mut bitmap = DecodedRegion::new(ctx.total_width, ctx.height_class_height);
        decode_bitmap_mmr(&mut bitmap, bitmap_data)?;
        bitmap
    };

    // Finally, we simply chop up the collective bitmap into its constituent
    // symbols.
    let mut x_offset: u32 = 0;
    for i in ctx.height_class_first_symbol..ctx.symbols_decoded_count {
        let sym_width = ctx.symbol_widths[i as usize];
        let mut symbol = DecodedRegion::new(sym_width, ctx.height_class_height);

        for y in 0..ctx.height_class_height {
            for x in 0..sym_width {
                let pixel = collective_bitmap.get_pixel(x_offset + x, y);
                symbol.set_pixel(x, y, pixel);
            }
        }

        ctx.new_symbols.push(symbol);
        x_offset += sym_width;
    }

    Ok(())
}

/// Exported symbols (6.5.10).
fn export_symbols(ctx: &mut SymbolDecodeContext<'_>) -> Result<Vec<DecodedRegion>> {
    let total_symbols = ctx.total_symbols();

    let mut read_run_length = || -> Result<u32> {
        let value = if ctx.header.flags.use_huffman {
            ctx.h_ctx
                .export_run_length_table
                .decode(&mut ctx.h_ctx.reader)?
        } else {
            ctx.a_ctx
                .export_run_length_decoder
                .decode(&mut ctx.a_ctx.decoder)
        }
        .ok_or(HuffmanError::UnexpectedOob)?;

        u32::try_from(value).map_err(|_| HuffmanError::InvalidCode.into())
    };
    let mut exported = Vec::with_capacity(ctx.header.num_exported_symbols as usize);
    let mut index: u32 = 0;
    let mut should_export = false;

    while index < total_symbols {
        let run_length = read_run_length()?;

        if index + run_length > total_symbols {
            bail!(SymbolError::OutOfRange);
        }

        if should_export {
            for i in index..index + run_length {
                let symbol = if i < ctx.num_input_symbols {
                    ctx.input_symbols[i as usize].clone()
                } else {
                    ctx.new_symbols[i as usize - ctx.num_input_symbols as usize].clone()
                };
                exported.push(symbol);
            }
        }

        index += run_length;
        should_export = !should_export;
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
