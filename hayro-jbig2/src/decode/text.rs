//! Text region segment parsing and decoding (7.4.3, 6.4).

use alloc::vec;
use alloc::vec::Vec;
use core::iter;

use super::generic_refinement::decode_bitmap;
use super::{
    AdaptiveTemplatePixel, CombinationOperator, RefinementTemplate, RegionSegmentInfo,
    parse_refinement_at_pixels, parse_region_segment_info,
};
use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::error::{DecodeError, HuffmanError, ParseError, Result, SymbolError, bail};
use crate::huffman_table::{HuffmanTable, StandardHuffmanTables, TableLine};
use crate::integer_decoder::IntegerDecoder;
use crate::reader::Reader;
use crate::symbol_id_decoder::SymbolIdDecoder;

/// Decode a text region segment (6.4).
pub(crate) fn decode(
    reader: &mut Reader<'_>,
    symbols: &[&DecodedRegion],
    referred_tables: &[HuffmanTable],
    standard_tables: &StandardHuffmanTables,
) -> Result<DecodedRegion> {
    let header = parse(reader, symbols.len() as u32)?;

    if header.flags.use_huffman {
        let ctx = DecodeContext::new_huffman(reader, &header, referred_tables, standard_tables)?;
        decode_with(ctx, symbols, &header)
    } else {
        let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;
        let mut decoder = ArithmeticDecoder::new(data);

        let num_symbols = symbols.len() as u32;
        let symbol_code_length = 32 - num_symbols.saturating_sub(1).leading_zeros();
        let mut contexts = TextRegionContexts::new(symbol_code_length);

        let num_gr_contexts = 1 << header.flags.refinement_template.context_bits();
        let mut gr_contexts = vec![Context::default(); num_gr_contexts];

        let ctx = DecodeContext::new_arithmetic(&mut decoder, &mut contexts, &mut gr_contexts);
        decode_with(ctx, symbols, &header)
    }
}

/// Decode a text region segment with a decode context (6.4).
pub(crate) fn decode_with(
    mut ctx: DecodeContext<'_, '_>,
    symbols: &[&DecodedRegion],
    header: &TextRegionHeader,
) -> Result<DecodedRegion> {
    let mut region = DecodedRegion::new(header.region_info.width, header.region_info.height);
    region.x_location = header.region_info.x_location;
    region.y_location = header.region_info.y_location;
    region.combination_operator = header.region_info.combination_operator;

    let strip_size = header.strip_size();

    if header.flags.default_pixel {
        for pixel in &mut region.data {
            *pixel = true;
        }
    }

    // "2) Decode the initial STRIPT value as described in 6.4.6." (6.4.5)
    let initial_strip_t = ctx.read_strip_delta_t(strip_size)?;
    let mut strip_t: i32 = initial_strip_t.checked_neg().ok_or(DecodeError::Overflow)?;
    let mut first_s: i32 = 0;
    let mut instance_count: u32 = 0;

    // "4) Decode each strip as follows:" (6.4.5)
    while instance_count < header.num_instances {
        // "b) Decode the strip's delta T value as described in 6.4.6."
        let delta_t = ctx.read_strip_delta_t(strip_size)?;
        strip_t = strip_t.checked_add(delta_t).ok_or(DecodeError::Overflow)?;

        // "c) Decode each symbol instance in the strip as follows:"
        let mut first_symbol_in_strip = true;
        let mut current_s: i32 = 0;

        loop {
            // "i) First symbol S coordinate / ii) Subsequent symbol S coordinate"
            if first_symbol_in_strip {
                let delta_first_s = ctx.read_first_s()?;
                first_s = first_s
                    .checked_add(delta_first_s)
                    .ok_or(DecodeError::Overflow)?;
                current_s = first_s;
                first_symbol_in_strip = false;
            } else {
                match ctx.read_delta_s()? {
                    Some(delta_s) => {
                        current_s = current_s
                            .checked_add(delta_s)
                            .and_then(|v| v.checked_add(header.flags.delta_s_offset as i32))
                            .ok_or(DecodeError::Overflow)?;
                    }
                    None => {
                        // OOB - end of strip.
                        break;
                    }
                }
            }

            // "iii) Decode the symbol instance's T coordinate."
            let current_t = ctx.read_symbol_t(strip_size, header.flags.log_strip_size)?;
            let symbol_t = strip_t
                .checked_add(current_t)
                .ok_or(DecodeError::Overflow)?;

            // "iv) Decode the symbol instance's symbol ID."
            let symbol_id = ctx.read_symbol_id()?;

            // "v) Determine the symbol instance's bitmap IB_I as described in 6.4.11."
            let (symbol_bitmap, symbol_width, symbol_height): (SymbolBitmap, i32, i32) =
                if !header.flags.use_refinement {
                    // "If SBREFINE is 0, then set R_I to 0." (6.4.11)
                    let symbol = symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;
                    (
                        SymbolBitmap::Reference(symbol_id),
                        symbol.width as i32,
                        symbol.height as i32,
                    )
                } else {
                    let refinement_flag = ctx.read_refinement_flag()?;

                    if refinement_flag == 0 {
                        let symbol = symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;
                        (
                            SymbolBitmap::Reference(symbol_id),
                            symbol.width as i32,
                            symbol.height as i32,
                        )
                    } else {
                        // Refinement decoding (6.4.11).
                        let reference_bitmap =
                            symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;
                        let reference_width = reference_bitmap.width;
                        let reference_height = reference_bitmap.height;

                        let rdw = ctx.read_refinement_delta_width()?;
                        let rdh = ctx.read_refinement_delta_height()?;
                        let rdx = ctx.read_refinement_x_offset()?;
                        let rdy = ctx.read_refinement_y_offset()?;

                        let refined_width = (reference_width as i32)
                            .checked_add(rdw)
                            .ok_or(DecodeError::Overflow)?
                            as u32;
                        let refined_height = (reference_height as i32)
                            .checked_add(rdh)
                            .ok_or(DecodeError::Overflow)?
                            as u32;
                        let reference_x_offset = rdw
                            .div_euclid(2)
                            .checked_add(rdx)
                            .ok_or(DecodeError::Overflow)?;
                        let reference_y_offset = rdh
                            .div_euclid(2)
                            .checked_add(rdy)
                            .ok_or(DecodeError::Overflow)?;

                        let mut refined = DecodedRegion::new(refined_width, refined_height);

                        ctx.decode_refinement_bitmap(
                            &mut refined,
                            reference_bitmap,
                            reference_x_offset,
                            reference_y_offset,
                            header.flags.refinement_template,
                            &header.refinement_at_pixels,
                        )?;

                        (
                            SymbolBitmap::Owned(refined),
                            refined_width as i32,
                            refined_height as i32,
                        )
                    }
                };

            let symbol_bitmap_ref: &DecodedRegion = match &symbol_bitmap {
                SymbolBitmap::Reference(idx) => symbols.get(*idx).ok_or(SymbolError::OutOfRange)?,
                SymbolBitmap::Owned(region) => region,
            };

            // "vi) Update CURS as follows:" (6.4.5)
            if !header.flags.transposed
                && (header.flags.reference_corner == ReferenceCorner::TopRight
                    || header.flags.reference_corner == ReferenceCorner::BottomRight)
            {
                current_s = current_s
                    .checked_add(symbol_width - 1)
                    .ok_or(DecodeError::Overflow)?;
            } else if header.flags.transposed
                && (header.flags.reference_corner == ReferenceCorner::BottomLeft
                    || header.flags.reference_corner == ReferenceCorner::BottomRight)
            {
                current_s = current_s
                    .checked_add(symbol_height - 1)
                    .ok_or(DecodeError::Overflow)?;
            }

            // "vii) Set: S_I = CURS"
            let symbol_s = current_s;

            // "viii) Determine the location of the symbol instance bitmap."
            let (x, y) = compute_symbol_location(
                symbol_s,
                symbol_t,
                symbol_width,
                symbol_height,
                header.flags.transposed,
                header.flags.reference_corner,
            );

            // "x) Draw IB_I into SBREG."
            region.combine(symbol_bitmap_ref, x, y, header.flags.combination_operator);

            // "xi) Update CURS as follows:" (6.4.5)
            if !header.flags.transposed
                && (header.flags.reference_corner == ReferenceCorner::TopLeft
                    || header.flags.reference_corner == ReferenceCorner::BottomLeft)
            {
                current_s = current_s
                    .checked_add(symbol_width - 1)
                    .ok_or(DecodeError::Overflow)?;
            } else if header.flags.transposed
                && (header.flags.reference_corner == ReferenceCorner::TopLeft
                    || header.flags.reference_corner == ReferenceCorner::TopRight)
            {
                current_s = current_s
                    .checked_add(symbol_height - 1)
                    .ok_or(DecodeError::Overflow)?;
            }

            // "xii) Set: NINSTANCES = NINSTANCES + 1"
            instance_count += 1;
        }
    }

    Ok(region)
}

/// Shared integer decoder contexts for text region decoding.
pub(crate) struct TextRegionContexts {
    /// IADT: Strip delta T decoder (6.4.6)
    pub(crate) iadt: IntegerDecoder,
    /// IAFS: First symbol S coordinate decoder (6.4.7)
    pub(crate) iafs: IntegerDecoder,
    /// IADS: Subsequent symbol S coordinate decoder (6.4.8)
    pub(crate) iads: IntegerDecoder,
    /// IAIT: Symbol instance T coordinate decoder (6.4.9)
    pub(crate) iait: IntegerDecoder,
    /// IAID: Symbol ID decoder (6.4.10)
    pub(crate) iaid: SymbolIdDecoder,
    /// IARI: Refinement image indicator decoder (6.4.11)
    pub(crate) iari: IntegerDecoder,
    /// IARDW: Refinement delta width decoder (6.4.11.1)
    pub(crate) iardw: IntegerDecoder,
    /// IARDH: Refinement delta height decoder (6.4.11.2)
    pub(crate) iardh: IntegerDecoder,
    /// IARDX: Refinement X offset decoder (6.4.11.3)
    pub(crate) iardx: IntegerDecoder,
    /// IARDY: Refinement Y offset decoder (6.4.11.4)
    pub(crate) iardy: IntegerDecoder,
}

impl TextRegionContexts {
    /// Create new text region contexts with the given symbol code length.
    pub(crate) fn new(symbol_code_length: u32) -> Self {
        Self {
            iadt: IntegerDecoder::new(),
            iafs: IntegerDecoder::new(),
            iads: IntegerDecoder::new(),
            iait: IntegerDecoder::new(),
            iaid: SymbolIdDecoder::new(symbol_code_length),
            iari: IntegerDecoder::new(),
            iardw: IntegerDecoder::new(),
            iardh: IntegerDecoder::new(),
            iardx: IntegerDecoder::new(),
            iardy: IntegerDecoder::new(),
        }
    }
}

pub(crate) enum DecodeContext<'a, 'b> {
    Huffman {
        reader: &'a mut Reader<'b>,
        tables: TextRegionHuffmanTables<'a>,
        symbol_codes: &'a HuffmanTable,
    },
    Arithmetic {
        decoder: &'a mut ArithmeticDecoder<'b>,
        contexts: &'a mut TextRegionContexts,
        gr_contexts: &'a mut [Context],
    },
}

impl<'a, 'b> DecodeContext<'a, 'b> {
    pub(crate) fn new_huffman(
        reader: &'a mut Reader<'b>,
        header: &'a TextRegionHeader,
        referred_tables: &'a [HuffmanTable],
        standard_tables: &'a StandardHuffmanTables,
    ) -> Result<Self> {
        let huffman_flags = header
            .huffman_flags
            .as_ref()
            .ok_or(HuffmanError::InvalidSelection)?;
        let tables = select_huffman_tables(huffman_flags, referred_tables, standard_tables)?;
        let symbol_codes = header
            .symbol_id_table
            .as_ref()
            .ok_or(HuffmanError::MissingTables)?;

        Ok(DecodeContext::Huffman {
            reader,
            tables,
            symbol_codes,
        })
    }

    pub(crate) fn new_arithmetic(
        decoder: &'a mut ArithmeticDecoder<'b>,
        contexts: &'a mut TextRegionContexts,
        gr_contexts: &'a mut [Context],
    ) -> Self {
        DecodeContext::Arithmetic {
            decoder,
            contexts,
            gr_contexts,
        }
    }

    fn read_strip_delta_t(&mut self, strip_size: u32) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                Ok(tables.delta_t.decode_no_oob(reader)? * strip_size as i32)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => {
                let value = contexts
                    .iadt
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;
                Ok(value * strip_size as i32)
            }
        }
    }

    fn read_first_s(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => tables.first_s.decode_no_oob(reader),
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iafs
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange.into()),
        }
    }

    fn read_delta_s(&mut self) -> Result<Option<i32>> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => tables.delta_s.decode(reader),
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => Ok(contexts.iads.decode(decoder)),
        }
    }

    fn read_symbol_t(&mut self, strip_size: u32, log_strip_size: u8) -> Result<i32> {
        if strip_size == 1 {
            return Ok(0);
        }

        match self {
            DecodeContext::Huffman { reader, .. } => reader
                .read_bits(log_strip_size)
                .ok_or(HuffmanError::InvalidCode.into())
                .map(|v| v as i32),
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iait
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange.into()),
        }
    }

    fn read_symbol_id(&mut self) -> Result<usize> {
        match self {
            DecodeContext::Huffman {
                reader,
                symbol_codes,
                ..
            } => symbol_codes.decode_no_oob(reader).map(|v| v as usize),
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => Ok(contexts.iaid.decode(decoder) as usize),
        }
    }

    fn read_refinement_flag(&mut self) -> Result<u8> {
        match self {
            DecodeContext::Huffman { reader, .. } => {
                reader.read_bit().ok_or(ParseError::UnexpectedEof.into())
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => Ok(contexts
                .iari
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange)? as u8),
        }
    }

    fn read_refinement_delta_width(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                tables.refinement_width.decode_no_oob(reader)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iardw
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange.into()),
        }
    }

    fn read_refinement_delta_height(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                tables.refinement_height.decode_no_oob(reader)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iardh
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange.into()),
        }
    }

    fn read_refinement_x_offset(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                tables.refinement_x.decode_no_oob(reader)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iardx
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange.into()),
        }
    }

    fn read_refinement_y_offset(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                tables.refinement_y.decode_no_oob(reader)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iardy
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange.into()),
        }
    }

    fn decode_refinement_bitmap(
        &mut self,
        refined: &mut DecodedRegion,
        reference_bitmap: &DecodedRegion,
        reference_x_offset: i32,
        reference_y_offset: i32,
        refinement_template: RefinementTemplate,
        refinement_at_pixels: &[AdaptiveTemplatePixel],
    ) -> Result<()> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                let refinement_data_size = tables.refinement_size.decode_no_oob(reader)? as u32;
                reader.align();

                let refinement_data = reader
                    .read_bytes(refinement_data_size as usize)
                    .ok_or(ParseError::UnexpectedEof)?;

                let mut temp_decoder = ArithmeticDecoder::new(refinement_data);
                let num_context_bits = refinement_template.context_bits();
                let mut temp_contexts = vec![Context::default(); 1 << num_context_bits];

                decode_bitmap(
                    &mut temp_decoder,
                    &mut temp_contexts,
                    refined,
                    reference_bitmap,
                    reference_x_offset,
                    reference_y_offset,
                    refinement_template,
                    refinement_at_pixels,
                    false,
                )
            }
            DecodeContext::Arithmetic {
                decoder,
                gr_contexts,
                ..
            } => decode_bitmap(
                decoder,
                gr_contexts,
                refined,
                reference_bitmap,
                reference_x_offset,
                reference_y_offset,
                refinement_template,
                refinement_at_pixels,
                false,
            ),
        }
    }
}

/// Result of determining a symbol instance bitmap.
enum SymbolBitmap {
    /// Use the symbol at this index directly (`R_I` = 0).
    Reference(usize),
    /// Use this refined bitmap (`R_I` = 1).
    Owned(DecodedRegion),
}

/// Compute the location of a symbol instance bitmap (6.4.5 step viii).
///
/// Returns (x, y) coordinates where the symbol should be placed.
fn compute_symbol_location(
    symbol_s: i32,
    symbol_t: i32,
    symbol_width: i32,
    symbol_height: i32,
    transposed: bool,
    reference_corner: ReferenceCorner,
) -> (i32, i32) {
    if !transposed {
        // "If TRANSPOSED is 0, then:"
        match reference_corner {
            // "If REFCORNER is TOPLEFT then the top left pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[S_I, T_I]."
            ReferenceCorner::TopLeft => (symbol_s, symbol_t),
            // "If REFCORNER is TOPRIGHT then the top right pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[S_I, T_I]."
            ReferenceCorner::TopRight => (symbol_s - symbol_width + 1, symbol_t),
            // "If REFCORNER is BOTTOMLEFT then the bottom left pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[S_I, T_I]."
            ReferenceCorner::BottomLeft => (symbol_s, symbol_t - symbol_height + 1),
            // "If REFCORNER is BOTTOMRIGHT then the bottom right pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[S_I, T_I]."
            ReferenceCorner::BottomRight => {
                (symbol_s - symbol_width + 1, symbol_t - symbol_height + 1)
            }
        }
    } else {
        // "If TRANSPOSED is 1, then:"
        match reference_corner {
            // "If REFCORNER is TOPLEFT then the top left pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[T_I, S_I]."
            ReferenceCorner::TopLeft => (symbol_t, symbol_s),
            // "If REFCORNER is TOPRIGHT then the top right pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[T_I, S_I]."
            ReferenceCorner::TopRight => (symbol_t - symbol_width + 1, symbol_s),
            // "If REFCORNER is BOTTOMLEFT then the bottom left pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[T_I, S_I]."
            ReferenceCorner::BottomLeft => (symbol_t, symbol_s - symbol_height + 1),
            // "If REFCORNER is BOTTOMRIGHT then the bottom right pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[T_I, S_I]."
            ReferenceCorner::BottomRight => {
                (symbol_t - symbol_width + 1, symbol_s - symbol_height + 1)
            }
        }
    }
}

/// Select Huffman tables based on flags (7.4.3.1.6).
fn select_huffman_tables<'a>(
    flags: &TextRegionHuffmanFlags,
    custom_tables: &'a [HuffmanTable],
    standard_tables: &'a StandardHuffmanTables,
) -> Result<TextRegionHuffmanTables<'a>> {
    let mut custom_table_idx = 0;

    let mut get_custom = || -> &'a HuffmanTable {
        let table = &custom_tables[custom_table_idx];
        custom_table_idx += 1;
        table
    };

    // "1) SBHUFFFS"
    let first_s = match flags.first_s_table {
        0 => standard_tables.table_f(),
        1 => standard_tables.table_g(),
        3 => get_custom(),
        _ => bail!(HuffmanError::InvalidSelection),
    };

    // "2) SBHUFFDS"
    let delta_s = match flags.delta_s_table {
        0 => standard_tables.table_h(),
        1 => standard_tables.table_i(),
        2 => standard_tables.table_j(),
        3 => get_custom(),
        _ => bail!(HuffmanError::InvalidSelection),
    };

    // "3) SBHUFFDT"
    let delta_t = match flags.delta_t_table {
        0 => standard_tables.table_k(),
        1 => standard_tables.table_l(),
        2 => standard_tables.table_m(),
        3 => get_custom(),
        _ => bail!(HuffmanError::InvalidSelection),
    };

    // "4) SBHUFFRDW"
    let refinement_width = match flags.refinement_width_table {
        0 => standard_tables.table_n(),
        1 => standard_tables.table_o(),
        3 => get_custom(),
        _ => bail!(HuffmanError::InvalidSelection),
    };

    // "5) SBHUFFRDH"
    let refinement_height = match flags.refinement_height_table {
        0 => standard_tables.table_n(),
        1 => standard_tables.table_o(),
        3 => get_custom(),
        _ => bail!(HuffmanError::InvalidSelection),
    };

    // "6) SBHUFFRDY"
    let refinement_y = match flags.refinement_y_table {
        0 => standard_tables.table_n(),
        1 => standard_tables.table_o(),
        3 => get_custom(),
        _ => bail!(HuffmanError::InvalidSelection),
    };

    // "7) SBHUFFRDX"
    let refinement_x = match flags.refinement_x_table {
        0 => standard_tables.table_n(),
        1 => standard_tables.table_o(),
        3 => get_custom(),
        _ => bail!(HuffmanError::InvalidSelection),
    };

    // "8) SBHUFFRSIZE"
    let refinement_size = match flags.refinement_size_table {
        0 => standard_tables.table_a(),
        1 => get_custom(),
        _ => bail!(HuffmanError::InvalidSelection),
    };

    Ok(TextRegionHuffmanTables {
        first_s,
        delta_s,
        delta_t,
        refinement_width,
        refinement_height,
        refinement_y,
        refinement_x,
        refinement_size,
    })
}

/// Decode the symbol ID Huffman table (7.4.3.1.7).
fn decode_symbol_id_huffman_table(
    reader: &mut Reader<'_>,
    num_symbols: u32,
) -> Result<HuffmanTable> {
    let mut runcode_lines: Vec<TableLine> = Vec::with_capacity(35);
    for runcode_idx in 0..35 {
        let prefix_length = reader.read_bits(4).ok_or(HuffmanError::InvalidCode)? as u8;
        runcode_lines.push(TableLine::new(runcode_idx, prefix_length, 0));
    }

    let runcode_table = HuffmanTable::build(&runcode_lines);
    let mut symbol_code_lengths = Vec::with_capacity(num_symbols as usize);

    while symbol_code_lengths.len() < num_symbols as usize {
        let runcode = runcode_table.decode_no_oob(reader)? as u32;

        match runcode {
            0..=31 => {
                symbol_code_lengths.push(runcode as u8);
            }
            32 => {
                // Copy previous 3-6 times.
                let extra_bits = reader.read_bits(2).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 3;
                let previous_length = *symbol_code_lengths
                    .last()
                    .ok_or(HuffmanError::InvalidCode)?;
                symbol_code_lengths.extend(iter::repeat_n(previous_length, repeat));
            }
            33 => {
                // Repeat 0 length 3-10 times.
                let extra_bits = reader.read_bits(3).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 3;
                symbol_code_lengths.extend(iter::repeat_n(0, repeat));
            }
            34 => {
                // Repeat 0 length 11-138 times.
                let extra_bits = reader.read_bits(7).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 11;
                symbol_code_lengths.extend(iter::repeat_n(0, repeat));
            }
            _ => bail!(HuffmanError::InvalidCode),
        }
    }

    if symbol_code_lengths.len() != num_symbols as usize {
        bail!(HuffmanError::InvalidCode);
    }

    reader.align();

    let symbol_lines: Vec<TableLine> = symbol_code_lengths
        .iter()
        .enumerate()
        .map(|(symbol_idx, &prefix_length)| TableLine::new(symbol_idx as i32, prefix_length, 0))
        .collect();
    Ok(HuffmanTable::build(&symbol_lines))
}

pub(crate) struct TextRegionHuffmanTables<'a> {
    first_s: &'a HuffmanTable,
    delta_s: &'a HuffmanTable,
    delta_t: &'a HuffmanTable,
    refinement_width: &'a HuffmanTable,
    refinement_height: &'a HuffmanTable,
    refinement_y: &'a HuffmanTable,
    refinement_x: &'a HuffmanTable,
    refinement_size: &'a HuffmanTable,
}

/// Reference corner for symbol placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceCorner {
    BottomLeft,
    TopLeft,
    BottomRight,
    TopRight,
}

impl ReferenceCorner {
    fn from_byte(value: u8) -> Self {
        match value {
            0 => Self::BottomLeft,
            1 => Self::TopLeft,
            2 => Self::BottomRight,
            3 => Self::TopRight,
            _ => unreachable!(),
        }
    }
}

/// Parsed text region segment flags (7.4.3.1.1).
#[derive(Debug, Clone)]
pub(crate) struct TextRegionFlags {
    pub(crate) use_huffman: bool,
    pub(crate) use_refinement: bool,
    pub(crate) log_strip_size: u8,
    pub(crate) reference_corner: ReferenceCorner,
    pub(crate) transposed: bool,
    pub(crate) combination_operator: CombinationOperator,
    pub(crate) default_pixel: bool,
    pub(crate) delta_s_offset: i8,
    pub(crate) refinement_template: RefinementTemplate,
}

/// Text region segment Huffman flags (7.4.3.1.2).
#[derive(Debug, Clone)]
pub(crate) struct TextRegionHuffmanFlags {
    pub(crate) first_s_table: u8,
    pub(crate) delta_s_table: u8,
    pub(crate) delta_t_table: u8,
    pub(crate) refinement_width_table: u8,
    pub(crate) refinement_height_table: u8,
    pub(crate) refinement_y_table: u8,
    pub(crate) refinement_x_table: u8,
    pub(crate) refinement_size_table: u8,
}

/// Parsed text region segment header (7.4.3.1).
#[derive(Debug, Clone)]
pub(crate) struct TextRegionHeader {
    pub(crate) region_info: RegionSegmentInfo,
    pub(crate) flags: TextRegionFlags,
    pub(crate) huffman_flags: Option<TextRegionHuffmanFlags>,
    pub(crate) refinement_at_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) num_instances: u32,
    pub(crate) symbol_id_table: Option<HuffmanTable>,
}

impl TextRegionHeader {
    pub(crate) fn strip_size(&self) -> u32 {
        1_u32 << self.flags.log_strip_size
    }
}

/// Parse text region segment flags (7.4.3.1.1).
fn parse_text_region_flags(reader: &mut Reader<'_>) -> Result<TextRegionFlags> {
    let flags_word = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let use_huffman = flags_word & 0x0001 != 0;
    let use_refinement = flags_word & 0x0002 != 0;
    let log_strip_size = ((flags_word >> 2) & 0x03) as u8;
    let reference_corner = ReferenceCorner::from_byte(((flags_word >> 4) & 0x03) as u8);
    let transposed = flags_word & 0x0040 != 0;
    let combination_operator = CombinationOperator::from_value(((flags_word >> 7) & 0x03) as u8)?;

    let default_pixel = flags_word & 0x0200 != 0;

    let delta_s_offset_raw = ((flags_word >> 10) & 0x1F) as u8;
    let delta_s_offset = if delta_s_offset_raw & 0x10 != 0 {
        (delta_s_offset_raw | 0xE0) as i8
    } else {
        delta_s_offset_raw as i8
    };

    let refinement_template = RefinementTemplate::from_byte((flags_word >> 15) as u8);

    Ok(TextRegionFlags {
        use_huffman,
        use_refinement,
        log_strip_size,
        reference_corner,
        transposed,
        combination_operator,
        default_pixel,
        delta_s_offset,
        refinement_template,
    })
}

/// Parse text region Huffman flags (7.4.3.1.2).
fn parse_text_region_huffman_flags(reader: &mut Reader<'_>) -> Result<TextRegionHuffmanFlags> {
    let flags_word = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let first_s_table = (flags_word & 0x03) as u8;
    let delta_s_table = ((flags_word >> 2) & 0x03) as u8;
    let delta_t_table = ((flags_word >> 4) & 0x03) as u8;
    let refinement_width_table = ((flags_word >> 6) & 0x03) as u8;
    let refinement_height_table = ((flags_word >> 8) & 0x03) as u8;
    let refinement_y_table = ((flags_word >> 10) & 0x03) as u8;
    let refinement_x_table = ((flags_word >> 12) & 0x03) as u8;
    let refinement_size_table = ((flags_word >> 14) & 0x01) as u8;

    Ok(TextRegionHuffmanFlags {
        first_s_table,
        delta_s_table,
        delta_t_table,
        refinement_width_table,
        refinement_height_table,
        refinement_y_table,
        refinement_x_table,
        refinement_size_table,
    })
}

/// Parse a text region segment header (7.4.3.1).
fn parse(reader: &mut Reader<'_>, num_symbols: u32) -> Result<TextRegionHeader> {
    let region_info = parse_region_segment_info(reader)?;
    let flags = parse_text_region_flags(reader)?;
    let huffman_flags = if flags.use_huffman {
        Some(parse_text_region_huffman_flags(reader)?)
    } else {
        None
    };

    let refinement_at_pixels =
        if flags.use_refinement && flags.refinement_template == RefinementTemplate::Template0 {
            parse_refinement_at_pixels(reader)?
        } else {
            Vec::new()
        };

    let num_instances = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    let symbol_id_table = if flags.use_huffman {
        Some(decode_symbol_id_huffman_table(reader, num_symbols)?)
    } else {
        None
    };

    Ok(TextRegionHeader {
        region_info,
        flags,
        huffman_flags,
        refinement_at_pixels,
        num_instances,
        symbol_id_table,
    })
}
