//! Symbol dictionary segment parsing and decoding (7.4.2, 6.5).

use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::decode::generic::{decode_bitmap_mmr, parse_adaptive_template_pixels};
use crate::decode::text::{
    DecodeContext, ReferenceCorner, TextRegionContexts, TextRegionFlags, TextRegionHeader,
    TextRegionHuffmanFlags, decode_with,
};
use crate::decode::{
    AdaptiveTemplatePixel, CombinationOperator, RefinementTemplate, RegionSegmentInfo, Template,
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
        symbols: Symbols::new(input_symbols, num_new_symbols as usize),
        symbol_widths: Vec::with_capacity(num_new_symbols as usize),
        height_class_first_symbol: 0,
        symbols_decoded_count: 0,
        total_width: 0,
        height_class_height: 0,
        header,
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

                    ctx.symbols.new.push(region);
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

                    ctx.symbols.new.push(symbol);
                }
            }

            ctx.symbols_decoded_count += 1;
        }

        if ctx.header.flags.use_huffman && !ctx.header.flags.use_refagg {
            // In case we have huffman coding and no refinement-aggregation, we use
            // the previously decoded symbol widths to decode the collective bitmap
            // and extract the individual symbols from that bitmap.
            decode_height_class_collective_bitmap(&mut ctx)?;
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
    // 6.5.8.2.1 Number of symbol instances in the aggregation.
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
    } else if aggregation_instance_count > 1 {
        decode_aggregation_bitmap(ctx, symbol_width, aggregation_instance_count as u32)
    } else {
        Err(DecodeError::Symbol(SymbolError::Invalid))
    }
}

/// Decode a refinement bitmap symbol with a single aggregate (6.5.8.2).
fn decode_refinement_bitmap(
    ctx: &mut SymbolDecodeContext<'_>,
    symbol_width: u32,
) -> Result<DecodedRegion> {
    let use_huffman = ctx.header.flags.use_huffman;
    let mut symbol_code_length = 32 - (ctx.total_symbols() - 1).leading_zeros();

    let (symbol_id, refinement_x_offset, refinement_y_offset) = if use_huffman {
        // See 6.5.8.2.3, the value should be at least 1 if we use huffman coding.
        symbol_code_length = symbol_code_length.max(1);

        let symbol_id = ctx
            .h_ctx
            .reader
            .read_bits(symbol_code_length as u8)
            .ok_or(ParseError::UnexpectedEof)? as usize;

        let refinement_x_offset = ctx
            .standard_tables
            .table_o()
            .decode(&mut ctx.h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)?;

        let refinement_y_offset = ctx
            .standard_tables
            .table_o()
            .decode(&mut ctx.h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)?;

        (symbol_id, refinement_x_offset, refinement_y_offset)
    } else {
        // Note that the contexts should be reused across multiple
        // bitmaps in the same symbol dictionary.
        let contexts = ctx
            .a_ctx
            .text_region_contexts
            .get_or_insert_with(|| TextRegionContexts::new(symbol_code_length));

        let symbol_id = contexts.iaid.decode(&mut ctx.a_ctx.decoder) as usize;

        let refinement_x_offset = contexts
            .iardx
            .decode(&mut ctx.a_ctx.decoder)
            .ok_or(SymbolError::UnexpectedOob)?;

        let refinement_y_offset = contexts
            .iardy
            .decode(&mut ctx.a_ctx.decoder)
            .ok_or(SymbolError::UnexpectedOob)?;

        (symbol_id, refinement_x_offset, refinement_y_offset)
    };

    let reference_region = ctx.symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;
    let mut region = DecodedRegion::new(symbol_width, ctx.height_class_height);

    if use_huffman {
        let bitmap_size = ctx
            .standard_tables
            .table_a()
            .decode(&mut ctx.h_ctx.reader)?
            .ok_or(HuffmanError::UnexpectedOob)? as usize;
        ctx.h_ctx.reader.align();

        let bitmap_data = ctx
            .h_ctx
            .reader
            .read_bytes(bitmap_size)
            .ok_or(ParseError::UnexpectedEof)?;

        let mut bitmap_decoder = ArithmeticDecoder::new(bitmap_data);

        generic_refinement::decode_bitmap(
            &mut bitmap_decoder,
            &mut ctx.a_ctx.refinement_region_contexts,
            &mut region,
            reference_region,
            refinement_x_offset,
            refinement_y_offset,
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
            refinement_x_offset,
            refinement_y_offset,
            ctx.header.flags.refinement_template,
            &ctx.header.refinement_at_pixels,
            false,
        )?;
    }

    Ok(region)
}

/// Decode an aggregation bitmap with more than one aggregate (6.5.8.2).
fn decode_aggregation_bitmap(
    ctx: &mut SymbolDecodeContext<'_>,
    symbol_width: u32,
    aggregation_instance_count: u32,
) -> Result<DecodedRegion> {
    let use_huffman = ctx.header.flags.use_huffman;

    // Concatenate input and new symbols.
    let mut all_symbols: Vec<&DecodedRegion> =
        Vec::with_capacity(ctx.symbols.input.len() + ctx.symbols.new.len());
    all_symbols.extend(ctx.symbols.input.iter().copied());
    for sym in &ctx.symbols.new {
        all_symbols.push(sym);
    }

    // Set all parameters according to Table 17.

    let symbol_code_length = 32 - (ctx.total_symbols() - 1).leading_zeros();

    let symbol_id_table = if use_huffman {
        Some(HuffmanTable::build_uniform(
            ctx.total_symbols(),
            symbol_code_length,
        ))
    } else {
        None
    };

    let huffman_flags = if use_huffman {
        Some(TextRegionHuffmanFlags {
            first_s_table: 0,
            delta_s_table: 0,
            delta_t_table: 0,
            refinement_width_table: 1,
            refinement_height_table: 1,
            refinement_y_table: 1,
            refinement_x_table: 1,
            refinement_size_table: 0,
        })
    } else {
        None
    };

    let header = TextRegionHeader {
        region_info: RegionSegmentInfo {
            width: symbol_width,
            height: ctx.height_class_height,
            x_location: 0,
            y_location: 0,
            combination_operator: CombinationOperator::Or,
            _colour_extension: false,
        },
        flags: TextRegionFlags {
            use_huffman,
            use_refinement: true,
            log_strip_size: 0,
            reference_corner: ReferenceCorner::TopLeft,
            transposed: false,
            combination_operator: CombinationOperator::Or,
            default_pixel: false,
            delta_s_offset: 0,
            refinement_template: ctx.header.flags.refinement_template,
        },
        huffman_flags,
        refinement_at_pixels: ctx.header.refinement_at_pixels.clone(),
        num_instances: aggregation_instance_count,
        symbol_id_table,
    };

    let decode_ctx = if use_huffman {
        DecodeContext::new_huffman(&mut ctx.h_ctx.reader, &header, &[], ctx.standard_tables)?
    } else {
        let contexts = ctx
            .a_ctx
            .text_region_contexts
            .get_or_insert_with(|| TextRegionContexts::new(symbol_code_length));

        DecodeContext::new_arithmetic(
            &mut ctx.a_ctx.decoder,
            contexts,
            &mut ctx.a_ctx.refinement_region_contexts,
        )
    };

    decode_with(decode_ctx, &all_symbols, &header)
}

struct Symbols<'a> {
    input: &'a [&'a DecodedRegion],
    new: Vec<DecodedRegion>,
}

impl<'a> Symbols<'a> {
    fn new(input: &'a [&'a DecodedRegion], capacity: usize) -> Self {
        Self {
            input,
            new: Vec::with_capacity(capacity),
        }
    }

    fn input_count(&self) -> u32 {
        self.input.len() as u32
    }

    fn get(&self, index: usize) -> Option<&DecodedRegion> {
        if index < self.input.len() {
            Some(self.input[index])
        } else {
            self.new.get(index - self.input.len())
        }
    }
}

struct SymbolDecodeContext<'a> {
    header: SymbolDictionaryHeader,
    a_ctx: ArithmeticContext<'a>,
    h_ctx: HuffmanContext<'a>,
    symbols: Symbols<'a>,
    standard_tables: &'a StandardHuffmanTables,
    symbol_widths: Vec<u32>,
    height_class_first_symbol: u32,
    symbols_decoded_count: u32,
    total_width: u32,
    height_class_height: u32,
}

impl SymbolDecodeContext<'_> {
    fn total_symbols(&self) -> u32 {
        self.symbols.input_count() + self.header.num_new_symbols
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

        let mut custom_table_idx = 0;
        let mut get_custom = || -> &HuffmanTable {
            let table = &referred_tables[custom_table_idx];
            custom_table_idx += 1;
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
fn decode_height_class_collective_bitmap(ctx: &mut SymbolDecodeContext<'_>) -> Result<()> {
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
    for symbol_idx in ctx.height_class_first_symbol..ctx.symbols_decoded_count {
        let symbol_width = ctx.symbol_widths[symbol_idx as usize];
        let mut symbol = DecodedRegion::new(symbol_width, ctx.height_class_height);

        for y in 0..ctx.height_class_height {
            for x in 0..symbol_width {
                let pixel = collective_bitmap.get_pixel(x_offset + x, y);
                symbol.set_pixel(x, y, pixel);
            }
        }

        ctx.symbols.new.push(symbol);
        x_offset = x_offset
            .checked_add(symbol_width)
            .ok_or(DecodeError::Overflow)?;
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

        let end_index = index.checked_add(run_length).ok_or(DecodeError::Overflow)?;
        if end_index > total_symbols {
            bail!(SymbolError::OutOfRange);
        }

        if should_export {
            for symbol_idx in index..end_index {
                let symbol = ctx
                    .symbols
                    .get(symbol_idx as usize)
                    .ok_or(SymbolError::OutOfRange)?
                    .clone();
                exported.push(symbol);
            }
        }

        index = end_index;
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
