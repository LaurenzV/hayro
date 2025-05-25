use crate::filter::jbig2::bitmap::decode_bitmap;
use crate::filter::jbig2::{Bitmap, DecodingContext, Jbig2Error, TemplatePixel, decode_mmr_bitmap, log2, Reader, print_bitmap};

// Halftone region decoding - ported from decodeHalftoneRegion function
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_halftone_region(
    mmr: bool,
    patterns: &[Bitmap],
    template: usize,
    region_width: usize,
    region_height: usize,
    default_pixel_value: u8,
    enable_skip: bool,
    combination_operator: u8,
    grid_width: usize,
    grid_height: usize,
    grid_offset_x: i32,
    grid_offset_y: i32,
    grid_vector_x: i32,
    grid_vector_y: i32,
    decoding_context: &mut DecodingContext,
) -> Result<Bitmap, Jbig2Error> {
    if enable_skip {
        return Err(Jbig2Error::new("skip is not supported"));
    }
    if combination_operator != 0 {
        return Err(Jbig2Error::new(&format!(
            "operator \"{}\" is not supported in halftone region",
            combination_operator
        )));
    }

    // Prepare bitmap
    let mut region_bitmap: Vec<Vec<u8>> = Vec::with_capacity(region_height);
    for _ in 0..region_height {
        let mut row = vec![0u8; region_width];
        if default_pixel_value != 0 {
            row.fill(default_pixel_value);
        }
        region_bitmap.push(row);
    }

    let number_of_patterns = patterns.len();

    let pattern0 = &patterns[0];
    let pattern_width = pattern0[0].len();
    let pattern_height = pattern0.len();
    let bits_per_value = log2(number_of_patterns);

    let mut at = Vec::new();
    if !mmr {
        at.push(TemplatePixel {
            x: if template <= 1 { 3 } else { 2 },
            y: -1,
        });
        if template == 0 {
            at.push(TemplatePixel { x: -3, y: -1 });
            at.push(TemplatePixel { x: 2, y: -2 });
            at.push(TemplatePixel { x: -2, y: -2 });
        }
    }

    // Annex C. Gray-scale Image Decoding Procedure
    let mut gray_scale_bit_planes = Vec::with_capacity(bits_per_value);
    let decoding_data = decoding_context.data.clone();
    
    let mmr_input = if mmr {
        Some(Reader::new(&decoding_data, decoding_context.start, decoding_context.end))
    }   else {
        None
    };
    
    for _i in (0..bits_per_value) {
        let bitmap = if mmr {
            // MMR bit planes are in one continuous stream. Only EOFB codes indicate
            // the end of each bitmap, so EOFBs must be decoded.
            decode_mmr_bitmap(
                mmr_input.as_ref().unwrap(),
                grid_width,
                grid_height,
                true, // end_of_block = true for bit planes
            )?
        } else {
            decode_bitmap(
                false,
                grid_width,
                grid_height,
                template,
                false,
                None,
                &at,
                decoding_context,
            )?
        };
        // print_bitmap(&bitmap);
        gray_scale_bit_planes.push(bitmap);
    }
    
    gray_scale_bit_planes.reverse();

    // 6.6.5.2 Rendering the patterns
    for mg in 0..grid_height {
        for ng in 0..grid_width {
            let mut bit = 0u8;
            let mut pattern_index = 0usize;

            // Gray decoding - extract pattern index from bit planes
            for j in (0..bits_per_value).rev() {
                // println!("{:?}", gray_scale_bit_planes[j][mg][ng]);
                bit ^= gray_scale_bit_planes[j][mg][ng]; // Gray decoding
                pattern_index |= (bit as usize) << j;
            }

            let pattern_bitmap = &patterns[pattern_index];
            // print_bitmap(pattern_bitmap);

            let x =
                (grid_offset_x + (mg as i32) * grid_vector_y + (ng as i32) * grid_vector_x) >> 8;
            let y =
                (grid_offset_y + (mg as i32) * grid_vector_x - (ng as i32) * grid_vector_y) >> 8;

            // Draw pattern bitmap at (x, y)
            if x >= 0
                && x + (pattern_width as i32) <= region_width as i32
                && y >= 0
                && y + (pattern_height as i32) <= region_height as i32
            {
                for i in 0..pattern_height {
                    let region_y = (y + i as i32) as usize;
                    let pattern_row = &pattern_bitmap[i];
                    let region_row = &mut region_bitmap[region_y];
                    for j in 0..pattern_width {
                        let region_x = (x + j as i32) as usize;
                        region_row[region_x] |= pattern_row[j];
                    }
                    // println!("{:?}", region_row);
                    // continue;
                }
            } else {
                // Bounds-checked path: pattern may be partially outside
                for i in 0..pattern_height {
                    let region_y = y + i as i32;
                    if region_y < 0 || region_y >= region_height as i32 {
                        continue;
                    }
                    let region_row = &mut region_bitmap[region_y as usize];
                    let pattern_row = &pattern_bitmap[i];
                    for j in 0..pattern_width {
                        let region_x = x + j as i32;
                        if region_x >= 0 && (region_x as usize) < region_width {
                            region_row[region_x as usize] |= pattern_row[j];
                        }
                    }
                }
            }
        }
    }

    Ok(region_bitmap)
}
