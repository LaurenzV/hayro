//! Text region segment parsing and decoding (7.4.3, 6.4).

use alloc::vec;
use alloc::vec::Vec;

use super::generic_refinement::decode_bitmap;
use super::{
    AdaptiveTemplatePixel, CombinationOperator, RefinementTemplate, RegionSegmentInfo,
    parse_refinement_at_pixels, parse_region_segment_info,
};
use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::error::{HuffmanError, ParseError, Result, SymbolError, bail};
use crate::huffman_table::{HuffmanTable, StandardHuffmanTables, TableLine};
use crate::integer_decoder::IntegerDecoder;
use crate::reader::Reader;
use crate::symbol_id_decoder::SymbolIdDecoder;

pub(crate) enum CodingMode<'a, 'b> {
    Huffman {
        reader: &'a mut Reader<'b>,
        referred_tables: &'a [HuffmanTable],
        standard_tables: &'a StandardHuffmanTables,
    },
    Arithmetic {
        decoder: &'a mut ArithmeticDecoder<'b>,
        contexts: &'a mut TextRegionContexts,
        gr_contexts: &'a mut [Context],
    },
}

/// Decode a text region segment (6.4).
pub(crate) fn decode(
    reader: &mut Reader<'_>,
    symbols: &[&DecodedRegion],
    referred_tables: &[HuffmanTable],
    standard_tables: &StandardHuffmanTables,
) -> Result<DecodedRegion> {
    let header = parse(reader, symbols.len() as u32)?;

    if header.flags.use_huffman {
        let coding = CodingMode::Huffman {
            reader,
            referred_tables,
            standard_tables,
        };
        decode_with(coding, symbols, &header)
    } else {
        let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;
        let mut decoder = ArithmeticDecoder::new(data);

        let num_symbols = symbols.len() as u32;
        let symbol_code_length = 32 - num_symbols.saturating_sub(1).leading_zeros();
        let mut contexts = TextRegionContexts::new(symbol_code_length);

        let num_gr_contexts = 1 << header.flags.refinement_template.context_bits();
        let mut gr_contexts = vec![Context::default(); num_gr_contexts];

        let coding = CodingMode::Arithmetic {
            decoder: &mut decoder,
            contexts: &mut contexts,
            gr_contexts: &mut gr_contexts,
        };
        decode_with(coding, symbols, &header)
    }
}

/// Decode a text region with an already-parsed header.
///
/// This is used both for normal text region segments and for aggregate symbol
/// decoding in symbol dictionaries (Table 17).
pub(crate) fn decode_with(
    coding: CodingMode<'_, '_>,
    symbols: &[&DecodedRegion],
    header: &TextRegionHeader,
) -> Result<DecodedRegion> {
    let mut region = match coding {
        CodingMode::Huffman {
            reader,
            referred_tables,
            standard_tables,
        } => decode_text_region_huffman(reader, symbols, header, referred_tables, standard_tables)?,
        CodingMode::Arithmetic {
            decoder,
            contexts,
            gr_contexts,
        } => {
            if header.flags.use_refinement {
                decode_text_region_refine_with_contexts(
                    decoder,
                    symbols,
                    header,
                    contexts,
                    gr_contexts,
                )?
            } else {
                decode_text_region_direct_with_contexts(decoder, symbols, header, contexts)?
            }
        }
    };

    region.x_location = header.region_info.x_location;
    region.y_location = header.region_info.y_location;
    region.combination_operator = header.region_info.combination_operator;

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

/// Decode text region without refinement (SBREFINE=0), using provided contexts.
fn decode_text_region_direct_with_contexts(
    decoder: &mut ArithmeticDecoder<'_>,
    symbols: &[&DecodedRegion],
    header: &TextRegionHeader,
    contexts: &mut TextRegionContexts,
) -> Result<DecodedRegion> {
    decode_text_region_with(
        decoder,
        symbols,
        header,
        contexts,
        |_decoder, symbol_id, _symbols, _contexts| {
            // "If SBREFINE is 0, then set R_I to 0." (6.4.11)
            // "If R_I is 0 then set the symbol instance bitmap IB_I to SBSYMS[ID_I]."
            Ok(SymbolBitmap::Reference(symbol_id))
        },
    )
}

/// Text region decoding with refinement, using provided contexts.
///
/// This variant allows sharing contexts across multiple calls, which is
/// required for symbol dictionary decoding (REFAGGNINST > 1).
fn decode_text_region_refine_with_contexts(
    decoder: &mut ArithmeticDecoder<'_>,
    symbols: &[&DecodedRegion],
    header: &TextRegionHeader,
    contexts: &mut TextRegionContexts,
    gr_contexts: &mut [Context],
) -> Result<DecodedRegion> {
    decode_text_region_with(
        decoder,
        symbols,
        header,
        contexts,
        |decoder, symbol_id, symbols, contexts| {
            // Decode R_I (refinement indicator)
            let refinement_flag = contexts
                .iari
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange)?;

            if refinement_flag == 0 {
                Ok(SymbolBitmap::Reference(symbol_id))
            } else {
                let reference_bitmap = symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;
                let reference_width = reference_bitmap.width;
                let reference_height = reference_bitmap.height;

                let refinement_delta_width = contexts
                    .iardw
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;
                let refinement_delta_height = contexts
                    .iardh
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;
                let refinement_x_offset = contexts
                    .iardx
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;
                let refinement_y_offset = contexts
                    .iardy
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;

                let refined_width = (reference_width as i32 + refinement_delta_width) as u32;
                let refined_height = (reference_height as i32 + refinement_delta_height) as u32;
                let reference_x_offset = refinement_delta_width.div_euclid(2) + refinement_x_offset;
                let reference_y_offset =
                    refinement_delta_height.div_euclid(2) + refinement_y_offset;

                let mut refined = DecodedRegion::new(refined_width, refined_height);
                decode_bitmap(
                    decoder,
                    gr_contexts,
                    &mut refined,
                    reference_bitmap,
                    reference_x_offset,
                    reference_y_offset,
                    header.flags.refinement_template,
                    &header.refinement_at_pixels,
                    false,
                )?;
                Ok(SymbolBitmap::Owned(refined))
            }
        },
    )
}

/// Result of determining a symbol instance bitmap.
pub(crate) enum SymbolBitmap {
    /// Use the symbol at this index directly (`R_I` = 0).
    Reference(usize),
    /// Use this refined bitmap (`R_I` = 1).
    Owned(DecodedRegion),
}

/// Core text region decoding loop (6.4.5).
///
/// Takes a closure that determines each symbol instance bitmap.
fn decode_text_region_with<F>(
    decoder: &mut ArithmeticDecoder<'_>,
    symbols: &[&DecodedRegion],
    header: &TextRegionHeader,
    contexts: &mut TextRegionContexts,
    mut get_symbol_bitmap: F,
) -> Result<DecodedRegion>
where
    F: FnMut(
        &mut ArithmeticDecoder<'_>,
        usize,
        &[&DecodedRegion],
        &mut TextRegionContexts,
    ) -> Result<SymbolBitmap>,
{
    let width = header.region_info.width;
    let height = header.region_info.height;
    let num_instances = header.num_instances;
    let strip_size = header.strip_size();
    let default_pixel = header.flags.default_pixel;
    let transposed = header.flags.transposed;
    let reference_corner = header.flags.reference_corner;
    let delta_s_offset = header.flags.delta_s_offset as i32;
    let combination_operator = header.flags.combination_operator;

    // "1) Fill a bitmap SBREG, of the size given by SBW and SBH, with the
    // SBDEFPIXEL value." (6.4.5)
    let mut region = DecodedRegion::new(width, height);

    if default_pixel {
        for pixel in &mut region.data {
            *pixel = true;
        }
    }

    // "2) Decode the initial STRIPT value as described in 6.4.6. Negate the
    // decoded value and assign this negated value to the variable STRIPT.
    // Assign the value 0 to FIRSTS. Assign the value 0 to NINSTANCES." (6.4.5)
    let initial_strip_t = decode_strip_delta_t(decoder, &mut contexts.iadt, strip_size)?;
    let mut strip_t: i32 = -initial_strip_t;
    let mut first_s: i32 = 0;
    let mut instance_count: u32 = 0;

    // "4) Decode each strip as follows:" (6.4.5)
    while instance_count < num_instances {
        // "a) If NINSTANCES is equal to SBNUMINSTANCES then there are no more
        // strips to decode, and the process of decoding the text region is
        // complete; proceed to step 5)." (6.4.5)
        // (checked by while condition)

        // "b) Decode the strip's delta T value as described in 6.4.6. Let DT be
        // the decoded value. Set: STRIPT = STRIPT + DT" (6.4.5)
        let delta_t = decode_strip_delta_t(decoder, &mut contexts.iadt, strip_size)?;
        strip_t += delta_t;

        // "c) Decode each symbol instance in the strip as follows:" (6.4.5)
        let mut first_symbol_in_strip = true;
        let mut current_s: i32 = 0;

        loop {
            // "i) If the current symbol instance is the first symbol instance in
            // the strip, then decode the first symbol instance's S coordinate as
            // described in 6.4.7. Let DFS be the decoded value. Set:
            //     FIRSTS = FIRSTS + DFS
            //     CURS = FIRSTS" (6.4.5)
            if first_symbol_in_strip {
                let delta_first_s = contexts
                    .iafs
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;
                first_s += delta_first_s;
                current_s = first_s;
                first_symbol_in_strip = false;
            } else {
                // "ii) Otherwise, if the current symbol instance is not the first
                // symbol instance in the strip, decode the symbol instance's S
                // coordinate as described in 6.4.8. If the result of this decoding
                // is OOB then the last symbol instance of the strip has been decoded;
                // proceed to step 3 d). Otherwise, let IDS be the decoded value. Set:
                //     CURS = CURS + IDS + SBDSOFFSET" (6.4.5)
                match contexts.iads.decode(decoder) {
                    Some(delta_s) => {
                        current_s = current_s + delta_s + delta_s_offset;
                    }
                    None => {
                        // OOB - end of strip
                        break;
                    }
                }
            }

            // "iii) Decode the symbol instance's T coordinate as described in 6.4.9.
            // Let CURT be the decoded value. Set: T_I = STRIPT + CURT" (6.4.5)
            let current_t = decode_symbol_t_coordinate(decoder, &mut contexts.iait, strip_size)?;
            let symbol_t = strip_t + current_t;

            // "iv) Decode the symbol instance's symbol ID as described in 6.4.10.
            // Let ID_I be the decoded value." (6.4.5)
            let symbol_id = contexts.iaid.decode(decoder) as usize;

            // "v) Determine the symbol instance's bitmap IB_I as described in 6.4.11.
            // The width and height of this bitmap shall be denoted as W_I and H_I
            // respectively." (6.4.5)
            let symbol_bitmap = get_symbol_bitmap(decoder, symbol_id, symbols, contexts)?;
            let (symbol_bitmap, symbol_width, symbol_height): (&DecodedRegion, i32, i32) =
                match &symbol_bitmap {
                    SymbolBitmap::Reference(symbol_idx) => {
                        let symbol = symbols.get(*symbol_idx).ok_or(SymbolError::OutOfRange)?;
                        (symbol, symbol.width as i32, symbol.height as i32)
                    }
                    SymbolBitmap::Owned(region) => {
                        (region, region.width as i32, region.height as i32)
                    }
                };

            // "vi) Update CURS as follows:" (6.4.5)
            // - If TRANSPOSED is 0, and REFCORNER is TOPRIGHT or BOTTOMRIGHT, set:
            //     CURS = CURS + W_I - 1
            // - If TRANSPOSED is 1, and REFCORNER is BOTTOMLEFT or BOTTOMRIGHT, set:
            //     CURS = CURS + H_I - 1
            // - Otherwise, do not change CURS in this step.
            if !transposed
                && (reference_corner == ReferenceCorner::TopRight
                    || reference_corner == ReferenceCorner::BottomRight)
            {
                current_s += symbol_width - 1;
            } else if transposed
                && (reference_corner == ReferenceCorner::BottomLeft
                    || reference_corner == ReferenceCorner::BottomRight)
            {
                current_s += symbol_height - 1;
            }

            // "vii) Set: S_I = CURS" (6.4.5)
            let symbol_s = current_s;

            // "viii) Determine the location of the symbol instance bitmap with
            // respect to SBREG as follows:" (6.4.5)
            let (x, y) = compute_symbol_location(
                symbol_s,
                symbol_t,
                symbol_width,
                symbol_height,
                transposed,
                reference_corner,
            );

            // "x) Draw IB_I into SBREG. Combine each pixel of IB_I with the current
            // value of the corresponding pixel in SBREG, using the combination
            // operator specified by SBCOMBOP. Write the results of each combination
            // into that pixel in SBREG." (6.4.5)
            draw_symbol(&mut region, symbol_bitmap, x, y, combination_operator);

            // "xi) Update CURS as follows:" (6.4.5)
            // - If TRANSPOSED is 0, and REFCORNER is TOPLEFT or BOTTOMLEFT, set:
            //     CURS = CURS + W_I - 1
            // - If TRANSPOSED is 1, and REFCORNER is TOPLEFT or TOPRIGHT, set:
            //     CURS = CURS + H_I - 1
            // - Otherwise, do not change CURS in this step.
            if !transposed
                && (reference_corner == ReferenceCorner::TopLeft
                    || reference_corner == ReferenceCorner::BottomLeft)
            {
                current_s += symbol_width - 1;
            } else if transposed
                && (reference_corner == ReferenceCorner::TopLeft
                    || reference_corner == ReferenceCorner::TopRight)
            {
                current_s += symbol_height - 1;
            }

            // "xii) Set: NINSTANCES = NINSTANCES + 1" (6.4.5)
            instance_count += 1;
        }
    }

    // "5) After all the strips have been decoded, the current contents of SBREG
    // are the results that shall be obtained by every decoder" (6.4.5)
    Ok(region)
}

/// Decode strip delta T (6.4.6).
///
/// "If SBHUFF is 0, decode a value using the IADT integer arithmetic decoding
/// procedure (see Annex A) and multiply the resulting value by SBSTRIPS." (6.4.6)
fn decode_strip_delta_t(
    decoder: &mut ArithmeticDecoder<'_>,
    iadt: &mut IntegerDecoder,
    strip_size: u32,
) -> Result<i32> {
    let value = iadt.decode(decoder).ok_or(SymbolError::OutOfRange)?;
    Ok(value * strip_size as i32)
}

/// Decode symbol instance T coordinate (6.4.9).
///
/// "If SBSTRIPS = 1, then the value decoded is always zero." (6.4.9)
/// "If SBHUFF is 0, decode a value using the IAIT integer arithmetic decoding
/// procedure (see Annex A)." (6.4.9)
fn decode_symbol_t_coordinate(
    decoder: &mut ArithmeticDecoder<'_>,
    iait: &mut IntegerDecoder,
    strip_size: u32,
) -> Result<i32> {
    if strip_size == 1 {
        // "NOTE – If SBSTRIPS = 1, then no bits are consumed, and the IAIT
        // integer arithmetic decoding procedure is never invoked." (6.4.9)
        Ok(0)
    } else {
        let value = iait.decode(decoder).ok_or(SymbolError::OutOfRange)?;
        Ok(value)
    }
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

/// Draw a symbol bitmap into the region using the specified combination operator.
fn draw_symbol(
    region: &mut DecodedRegion,
    symbol: &DecodedRegion,
    x: i32,
    y: i32,
    combination_operator: CombinationOperator,
) {
    for src_y in 0..symbol.height {
        let dest_y = y + src_y as i32;
        if dest_y < 0 || dest_y >= region.height as i32 {
            continue;
        }

        for src_x in 0..symbol.width {
            let dest_x = x + src_x as i32;
            if dest_x < 0 || dest_x >= region.width as i32 {
                continue;
            }

            let src_pixel = symbol.get_pixel(src_x, src_y);
            let dst_pixel = region.get_pixel(dest_x as u32, dest_y as u32);

            let result = match combination_operator {
                CombinationOperator::Or => dst_pixel | src_pixel,
                CombinationOperator::And => dst_pixel & src_pixel,
                CombinationOperator::Xor => dst_pixel ^ src_pixel,
                CombinationOperator::Xnor => !(dst_pixel ^ src_pixel),
                CombinationOperator::Replace => src_pixel,
            };

            region.set_pixel(dest_x as u32, dest_y as u32, result);
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

/// Decode a text region using Huffman coding (SBHUFF=1).
fn decode_text_region_huffman(
    reader: &mut Reader<'_>,
    symbols: &[&DecodedRegion],
    header: &TextRegionHeader,
    referred_tables: &[HuffmanTable],
    standard_tables: &StandardHuffmanTables,
) -> Result<DecodedRegion> {
    let huffman_flags = header
        .huffman_flags
        .as_ref()
        .ok_or(HuffmanError::InvalidSelection)?;

    let custom_count = [
        huffman_flags.first_s_table == 3,
        huffman_flags.delta_s_table == 3,
        huffman_flags.delta_t_table == 3,
        huffman_flags.refinement_width_table == 3,
        huffman_flags.refinement_height_table == 3,
        huffman_flags.refinement_y_table == 3,
        huffman_flags.refinement_x_table == 3,
        huffman_flags.refinement_size_table == 1,
    ]
    .into_iter()
    .filter(|x| *x)
    .count();

    if referred_tables.len() < custom_count {
        bail!(HuffmanError::MissingTables);
    }

    let tables = select_huffman_tables(huffman_flags, referred_tables, standard_tables)?;

    let symbol_codes = header
        .symbol_id_table
        .as_ref()
        .ok_or(HuffmanError::MissingTables)?;

    let width = header.region_info.width;
    let height = header.region_info.height;
    let num_instances = header.num_instances;
    let strip_size = header.strip_size();
    let default_pixel = header.flags.default_pixel;
    let transposed = header.flags.transposed;
    let reference_corner = header.flags.reference_corner;
    let delta_s_offset = header.flags.delta_s_offset as i32;
    let combination_operator = header.flags.combination_operator;
    let use_refinement = header.flags.use_refinement;
    let log_strip_size = header.flags.log_strip_size;

    // "1) Fill a bitmap SBREG, of the size given by SBW and SBH, with the
    // SBDEFPIXEL value." (6.4.5)
    let mut region = DecodedRegion::new(width, height);
    if default_pixel {
        for pixel in &mut region.data {
            *pixel = true;
        }
    }

    // "2) Decode the initial STRIPT value as described in 6.4.6." (6.4.5)
    // "If SBHUFF is 1, decode a value using the Huffman table specified by
    // SBHUFFDT and multiply the resulting value by SBSTRIPS." (6.4.6)
    let initial_strip_t = decode_huffman_value(tables.delta_t, reader)? * strip_size as i32;
    let mut strip_t: i32 = -initial_strip_t;
    let mut first_s: i32 = 0;
    let mut instance_count: u32 = 0;

    // "4) Decode each strip as follows:" (6.4.5)
    while instance_count < num_instances {
        // "b) Decode the strip's delta T value as described in 6.4.6."
        let dt = decode_huffman_value(tables.delta_t, reader)? * strip_size as i32;
        strip_t += dt;

        // "c) Decode each symbol instance in the strip"
        let mut first_symbol_in_strip = true;
        let mut current_s: i32 = 0;

        loop {
            if first_symbol_in_strip {
                // "i) First symbol instance's S coordinate (6.4.7)
                // If SBHUFF is 1, decode a value using the Huffman table
                // specified by SBHUFFFS." (6.4.7)
                let delta_first_s = decode_huffman_value(tables.first_s, reader)?;
                first_s += delta_first_s;
                current_s = first_s;
                first_symbol_in_strip = false;
            } else {
                // "ii) Subsequent symbol instance S coordinate (6.4.8)
                // If SBHUFF is 1, decode a value using the Huffman table
                // specified by SBHUFFDS." (6.4.8)
                let Some(delta_s) = tables.delta_s.decode(reader)? else {
                    // End of strip (OOB).
                    break;
                };

                current_s = current_s + delta_s + delta_s_offset;
            }

            // "iii) Symbol instance T coordinate (6.4.9)
            // If SBSTRIPS = 1, then the value decoded is always zero.
            // If SBHUFF is 1, decode a value by reading ceil(log2(SBSTRIPS))
            // bits directly from the bitstream." (6.4.9)
            let current_t = if strip_size == 1 {
                0
            } else {
                reader
                    .read_bits(log_strip_size)
                    .ok_or(HuffmanError::InvalidCode)? as i32
            };
            let symbol_t = strip_t + current_t;

            // "iv) Symbol instance symbol ID (6.4.10)
            // If SBHUFF is 1, decode a value by reading one bit at a time until
            // the resulting bit string is equal to one of the entries in
            // SBSYMCODES." (6.4.10)
            let symbol_id = decode_huffman_value(symbol_codes, reader)? as usize;

            // "v) Determine the symbol instance's bitmap IB_I as described in
            // 6.4.11." (6.4.5)
            let (symbol_bitmap, symbol_width, symbol_height): (
                alloc::borrow::Cow<'_, DecodedRegion>,
                i32,
                i32,
            ) = if !use_refinement {
                // "If SBREFINE is 0, then set R_I to 0." (6.4.11)
                let symbol = symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;
                (
                    alloc::borrow::Cow::Borrowed(*symbol),
                    symbol.width as i32,
                    symbol.height as i32,
                )
            } else {
                // "If SBREFINE is 1, then decode R_I as follows:
                // If SBHUFF is 1, then read one bit and set R_I to the value
                // of that bit." (6.4.11)
                let refinement_flag = reader.read_bit().ok_or(ParseError::UnexpectedEof)?;

                if refinement_flag == 0 {
                    let symbol = symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;
                    (
                        alloc::borrow::Cow::Borrowed(*symbol),
                        symbol.width as i32,
                        symbol.height as i32,
                    )
                } else {
                    // Refinement decoding (6.4.11)
                    let reference_bitmap = symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;
                    let reference_width = reference_bitmap.width;
                    let reference_height = reference_bitmap.height;

                    // "1) Decode the symbol instance refinement delta width"
                    let refinement_delta_width =
                        decode_huffman_value(tables.refinement_width, reader)?;

                    // "2) Decode the symbol instance refinement delta height"
                    let refinement_delta_height =
                        decode_huffman_value(tables.refinement_height, reader)?;

                    // "3) Decode the symbol instance refinement X offset"
                    let refinement_x_offset = decode_huffman_value(tables.refinement_x, reader)?;

                    // "4) Decode the symbol instance refinement Y offset"
                    let refinement_y_offset = decode_huffman_value(tables.refinement_y, reader)?;

                    // "5) If SBHUFF is 1, then:
                    // a) Decode the symbol instance refinement bitmap data size
                    // b) Skip over any bits remaining in the last byte read"
                    let refinement_data_size =
                        decode_huffman_value(tables.refinement_size, reader)? as u32;
                    reader.align();

                    // "6) Decode the refinement bitmap"
                    let refined_width = (reference_width as i32 + refinement_delta_width) as u32;
                    let refined_height = (reference_height as i32 + refinement_delta_height) as u32;
                    let reference_x_offset =
                        refinement_delta_width.div_euclid(2) + refinement_x_offset;
                    let reference_y_offset =
                        refinement_delta_height.div_euclid(2) + refinement_y_offset;

                    let mut refined = DecodedRegion::new(refined_width, refined_height);

                    // Read the refinement data (refinement_data_size bytes)
                    let refinement_data = reader
                        .read_bytes(refinement_data_size as usize)
                        .ok_or(ParseError::UnexpectedEof)?;

                    // Decode refinement bitmap from raw bytes.
                    // TPGRON is always 0 for text region refinements (Table 12).
                    let mut decoder = ArithmeticDecoder::new(refinement_data);
                    let num_context_bits = header.flags.refinement_template.context_bits();
                    let mut contexts = vec![Context::default(); 1 << num_context_bits];

                    decode_bitmap(
                        &mut decoder,
                        &mut contexts,
                        &mut refined,
                        reference_bitmap,
                        reference_x_offset,
                        reference_y_offset,
                        header.flags.refinement_template,
                        &header.refinement_at_pixels,
                        false, // TPGRON = 0
                    )?;

                    (
                        alloc::borrow::Cow::Owned(refined),
                        refined_width as i32,
                        refined_height as i32,
                    )
                }
            };

            // "vi) Update CURS as follows:"
            if !transposed
                && (reference_corner == ReferenceCorner::TopRight
                    || reference_corner == ReferenceCorner::BottomRight)
            {
                current_s += symbol_width - 1;
            } else if transposed
                && (reference_corner == ReferenceCorner::BottomLeft
                    || reference_corner == ReferenceCorner::BottomRight)
            {
                current_s += symbol_height - 1;
            }

            // "vii) Set: S_I = CURS"
            let symbol_s = current_s;

            // "viii) Determine the location"
            let (x, y) = compute_symbol_location(
                symbol_s,
                symbol_t,
                symbol_width,
                symbol_height,
                transposed,
                reference_corner,
            );

            // "x) Draw IB_I into SBREG"
            draw_symbol(&mut region, &symbol_bitmap, x, y, combination_operator);

            // "xi) Update CURS"
            if !transposed
                && (reference_corner == ReferenceCorner::TopLeft
                    || reference_corner == ReferenceCorner::BottomLeft)
            {
                current_s += symbol_width - 1;
            } else if transposed
                && (reference_corner == ReferenceCorner::TopLeft
                    || reference_corner == ReferenceCorner::TopRight)
            {
                current_s += symbol_height - 1;
            }

            // "xii) Set: NINSTANCES = NINSTANCES + 1"
            instance_count += 1;
        }
    }

    Ok(region)
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
        let runcode = decode_huffman_value(&runcode_table, reader)? as u32;

        // "4) Interpret the RUNCODE code and the additional bits (if any)
        // according to Table 29. This gives the symbol ID code lengths for
        // one or more symbols." (7.4.3.1.7)
        //
        // Table 32 – Meaning of the run codes:
        // RUNCODE0-31: Symbol ID code length is 0-31
        // RUNCODE32: Copy previous length 3-6 times (2 extra bits + 3)
        // RUNCODE33: Repeat 0 length 3-10 times (3 extra bits + 3)
        // RUNCODE34: Repeat 0 length 11-138 times (7 extra bits + 11)
        match runcode {
            0..=31 => {
                symbol_code_lengths.push(runcode as u8);
            }
            32 => {
                // Copy previous 3-6 times
                let extra_bits = reader.read_bits(2).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 3;
                let previous_length = *symbol_code_lengths
                    .last()
                    .ok_or(HuffmanError::InvalidCode)?;
                for _ in 0..repeat {
                    if symbol_code_lengths.len() >= num_symbols as usize {
                        break;
                    }
                    symbol_code_lengths.push(previous_length);
                }
            }
            33 => {
                // Repeat 0 length 3-10 times
                let extra_bits = reader.read_bits(3).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 3;
                for _ in 0..repeat {
                    if symbol_code_lengths.len() >= num_symbols as usize {
                        break;
                    }
                    symbol_code_lengths.push(0);
                }
            }
            34 => {
                // Repeat 0 length 11-138 times
                let extra_bits = reader.read_bits(7).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 11;
                for _ in 0..repeat {
                    if symbol_code_lengths.len() >= num_symbols as usize {
                        break;
                    }
                    symbol_code_lengths.push(0);
                }
            }
            _ => bail!(HuffmanError::InvalidCode),
        }
    }

    // "6) Skip over the remaining bits in the last byte read, so that the actual
    // text region decoding procedure begins on a byte boundary." (7.4.3.1.7)
    reader.align();

    // "7) Assign a Huffman code to each symbol by applying the algorithm in B.3
    // to the symbol ID code lengths just decoded. The result is the symbol ID
    // Huffman table SBSYMCODES." (7.4.3.1.7)
    let symbol_lines: Vec<TableLine> = symbol_code_lengths
        .iter()
        .enumerate()
        .map(|(symbol_idx, &prefix_length)| TableLine::new(symbol_idx as i32, prefix_length, 0))
        .collect();
    Ok(HuffmanTable::build(&symbol_lines))
}

/// Collection of Huffman tables for text region decoding.
struct TextRegionHuffmanTables<'a> {
    first_s: &'a HuffmanTable,
    delta_s: &'a HuffmanTable,
    delta_t: &'a HuffmanTable,
    refinement_width: &'a HuffmanTable,
    refinement_height: &'a HuffmanTable,
    refinement_y: &'a HuffmanTable,
    refinement_x: &'a HuffmanTable,
    refinement_size: &'a HuffmanTable,
}

/// Decode a value from a Huffman table, requiring a value (not OOB).
fn decode_huffman_value(table: &HuffmanTable, reader: &mut Reader<'_>) -> Result<i32> {
    Ok(table.decode(reader)?.ok_or(HuffmanError::InvalidCode)?)
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
    /// Symbol ID Huffman table (SBSYMCODES).
    /// For normal text regions, this is read from the stream (7.4.3.1.7).
    /// For aggregate decoding (Table 17), this is a fixed-width table (6.5.8.2.3).
    pub(crate) symbol_id_table: Option<HuffmanTable>,
}

impl TextRegionHeader {
    /// Compute SBSTRIPS from `log_strip_size`.
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
