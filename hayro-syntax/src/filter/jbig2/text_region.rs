use crate::filter::jbig2::refinement::decode_refinement;
use crate::filter::jbig2::{
    Bitmap, DecodingContext, Jbig2Error, Reader, TemplatePixel, TextRegionHuffmanTables,
};

// Text region decoding - ported from decodeTextRegion function
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_text_region(
    huffman: bool,
    refinement: bool,
    width: usize,
    height: usize,
    default_pixel_value: u8,
    number_of_symbol_instances: usize,
    strip_size: usize,
    input_symbols: &[Bitmap],
    symbol_code_length: usize,
    transposed: bool,
    ds_offset: i32,
    reference_corner: u8,
    combination_operator: u8,
    huffman_tables: Option<&TextRegionHuffmanTables>,
    refinement_template_index: usize,
    refinement_at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
    log_strip_size: usize,
    huffman_input: Option<&Reader>,
) -> Result<Bitmap, Jbig2Error> {
    if huffman && refinement {
        return Err(Jbig2Error::new("refinement with Huffman is not supported"));
    }

    // Prepare bitmap
    let mut bitmap = Vec::new();
    for _ in 0..height {
        let mut row = vec![0u8; width];
        if default_pixel_value != 0 {
            row.fill(default_pixel_value);
        }
        bitmap.push(row);
    }

    let mut strip_t = if huffman {
        -huffman_tables
            .unwrap()
            .table_delta_t
            .decode(huffman_input.unwrap())?
            .ok_or_else(|| Jbig2Error::new("Failed to decode initial stripT"))?
    } else {
        -decoding_context
            .decode_integer("IADT")
            .ok_or_else(|| Jbig2Error::new("Failed to decode initial stripT"))?
    };

    let mut first_s = 0i32;
    let mut i = 0;

    while i < number_of_symbol_instances {
        let delta_t = if huffman {
            huffman_tables
                .unwrap()
                .table_delta_t
                .decode(huffman_input.unwrap())?
                .ok_or_else(|| Jbig2Error::new("Failed to decode deltaT"))?
        } else {
            decoding_context
                .decode_integer("IADT")
                .ok_or_else(|| Jbig2Error::new("Failed to decode deltaT"))?
        };
        strip_t += delta_t;

        let delta_first_s = if huffman {
            huffman_tables
                .unwrap()
                .table_first_s
                .as_ref()
                .unwrap()
                .decode(huffman_input.unwrap())?
                .ok_or_else(|| Jbig2Error::new("Failed to decode deltaFirstS"))?
        } else {
            decoding_context
                .decode_integer("IAFS")
                .ok_or_else(|| Jbig2Error::new("Failed to decode deltaFirstS"))?
        };
        first_s += delta_first_s;
        let mut current_s = first_s;

        loop {
            let mut current_t = 0;
            if strip_size > 1 {
                current_t = if huffman {
                    huffman_input.unwrap().read_bits(log_strip_size)? as i32
                } else {
                    decoding_context
                        .decode_integer("IAIT")
                        .ok_or_else(|| Jbig2Error::new("Failed to decode currentT"))?
                };
            }

            let t = (strip_size as i32) * strip_t + current_t;

            let symbol_id = if huffman {
                match huffman_tables
                    .unwrap()
                    .symbol_id_table
                    .decode(huffman_input.unwrap())?
                {
                    Some(id) => id,
                    None => return Err(Jbig2Error::new("Unexpected OOB in symbolID decode")),
                }
            } else {
                decoding_context.decode_iaid(symbol_code_length) as i32
            };

            let apply_refinement = refinement
                && if huffman {
                    huffman_input.unwrap().read_bit()? != 0
                } else {
                    decoding_context
                        .decode_integer("IARI")
                        .ok_or_else(|| Jbig2Error::new("Failed to decode refinement flag"))?
                        != 0
                };

            let mut symbol_bitmap = &input_symbols[symbol_id as usize];
            let mut symbol_width = symbol_bitmap[0].len();
            let mut symbol_height = symbol_bitmap.len();
            let mut refined_bitmap_storage: Option<Bitmap> = None;

            if apply_refinement {
                let rdw = decoding_context
                    .decode_integer("IARDW")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode rdw"))?;
                let rdh = decoding_context
                    .decode_integer("IARDH")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode rdh"))?;
                let rdx = decoding_context
                    .decode_integer("IARDX")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode rdx"))?;
                let rdy = decoding_context
                    .decode_integer("IARDY")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode rdy"))?;

                symbol_width = (symbol_width as i32 + rdw) as usize;
                symbol_height = (symbol_height as i32 + rdh) as usize;

                let refined_bitmap = decode_refinement(
                    symbol_width,
                    symbol_height,
                    refinement_template_index,
                    symbol_bitmap,
                    (rdw >> 1) + rdx,
                    (rdh >> 1) + rdy,
                    false,
                    refinement_at,
                    decoding_context,
                )?;
                refined_bitmap_storage = Some(refined_bitmap);
                symbol_bitmap = refined_bitmap_storage.as_ref().unwrap();
            }

            let mut increment = 0;
            if !transposed {
                if reference_corner > 1 {
                    current_s += symbol_width as i32 - 1;
                } else {
                    increment = symbol_width as i32 - 1;
                }
            } else if (reference_corner & 1) == 0 {
                current_s += symbol_height as i32 - 1;
            } else {
                increment = symbol_height as i32 - 1;
            }

            let offset_t = t - if (reference_corner & 1) != 0 {
                0
            } else {
                symbol_height as i32 - 1
            };
            let offset_s = current_s
                - if (reference_corner & 2) != 0 {
                    symbol_width as i32 - 1
                } else {
                    0
                };

            if transposed {
                // Place Symbol Bitmap from T1,S1
                for s2 in 0..symbol_height {
                    let row_idx = (offset_s + s2 as i32) as usize;
                    if row_idx >= bitmap.len() {
                        continue;
                    }

                    let symbol_row = &symbol_bitmap[s2];
                    // To ignore Parts of Symbol bitmap which goes outside bitmap region
                    let max_width = ((width as i32) - offset_t).min(symbol_width as i32) as usize;

                    match combination_operator {
                        0 => {
                            // OR
                            for t2 in 0..max_width {
                                let col_idx = (offset_t + t2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() {
                                    bitmap[row_idx][col_idx] |= symbol_row[t2];
                                }
                            }
                        }
                        2 => {
                            // XOR
                            for t2 in 0..max_width {
                                let col_idx = (offset_t + t2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() {
                                    bitmap[row_idx][col_idx] ^= symbol_row[t2];
                                }
                            }
                        }
                        _ => {
                            return Err(Jbig2Error::new(&format!(
                                "operator {} is not supported",
                                combination_operator
                            )));
                        }
                    }
                }
            } else {
                for t2 in 0..symbol_height {
                    let row_idx = (offset_t + t2 as i32) as usize;
                    if row_idx >= bitmap.len() {
                        continue;
                    }

                    let symbol_row = &symbol_bitmap[t2];

                    match combination_operator {
                        0 => {
                            // OR
                            for s2 in 0..symbol_width {
                                let col_idx = (offset_s + s2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() {
                                    bitmap[row_idx][col_idx] |= symbol_row[s2];
                                }
                            }
                        }
                        2 => {
                            // XOR
                            for s2 in 0..symbol_width {
                                let col_idx = (offset_s + s2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() {
                                    bitmap[row_idx][col_idx] ^= symbol_row[s2];
                                }
                            }
                        }
                        _ => {
                            return Err(Jbig2Error::new(&format!(
                                "operator {} is not supported",
                                combination_operator
                            )));
                        }
                    }
                }
            }

            i += 1;
            let delta_s = if huffman {
                huffman_tables
                    .unwrap()
                    .table_delta_s
                    .decode(huffman_input.unwrap())?
            } else {
                decoding_context.decode_integer("IADS")
            };

            if delta_s.is_none() {
                break; // OOB
            }
            current_s += increment + delta_s.unwrap() + ds_offset;
        }
    }

    Ok(bitmap)
}
