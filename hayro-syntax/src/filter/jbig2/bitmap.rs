use crate::filter::jbig2::bitmap_template0::decode_bitmap_template0;
use crate::filter::jbig2::tables::{CODING_TEMPLATES, REUSED_CONTEXTS};
use crate::filter::jbig2::{Bitmap, DecodingContext, Jbig2Error, TemplatePixel, decode_mmr_bitmap, Reader};
use std::cell::RefCell;
use std::rc::Rc;

// 6.2 Generic Region Decoding Procedure - General case
pub(crate) fn decode_bitmap(
    mmr: bool,
    width: usize,
    height: usize,
    template_index: usize,
    prediction: bool,
    skip: Option<&Bitmap>,
    at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
) -> Result<Bitmap, Jbig2Error> {
    // println!("Decode bitmap: {}", decoding_context.decoder.counter);
    if mmr {
        let reader = Reader::new(&decoding_context.data, decoding_context.start, decoding_context.end);
        return decode_mmr_bitmap(&reader, width, height, false);
    }

    // Use optimized version for the most common case
    if template_index == 0
        && skip.is_none()
        && !prediction
        && at.len() == 4
        && at[0].x == 3
        && at[0].y == -1
        && at[1].x == -3
        && at[1].y == -1
        && at[2].x == 2
        && at[2].y == -2
        && at[3].x == -2
        && at[3].y == -2
    {
        return Ok(decode_bitmap_template0(width, height, decoding_context));
    }

    let useskip = skip.is_some();
    let mut template = CODING_TEMPLATES[template_index]
        .iter()
        .map(|[x, y]| TemplatePixel { x: *x, y: *y })
        .collect::<Vec<_>>();
    template.extend_from_slice(at);

    // Sorting is non-standard, and it is not required. But sorting increases
    // the number of template bits that can be reused from the previous
    // contextLabel in the main loop.
    template.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));

    let template_length = template.len();

    let mut template_x: Vec<i8> = vec![0; template_length];
    let mut template_y: Vec<i8> = vec![0; template_length];
    let mut changing_template_entries = Vec::new();
    let mut reuse_mask = 0u32;
    let mut min_x = 0i32;
    let mut max_x = 0i32;
    let mut min_y = 0i32;

    for k in 0..template_length {
        template_x[k] = template[k].x as i8;
        template_y[k] = template[k].y as i8;

        min_x = min_x.min(template[k].x);
        max_x = max_x.max(template[k].x);
        min_y = min_y.min(template[k].y);

        // Check if the template pixel appears in two consecutive context labels,
        // so it can be reused. Otherwise, we add it to the list of changing
        // template entries.
        if k < template_length - 1
            && template[k].y == template[k + 1].y
            && template[k].x == template[k + 1].x - 1
        {
            reuse_mask |= 1 << (template_length - 1 - k);
        } else {
            changing_template_entries.push(k);
        }
    }

    let changing_entries_length = changing_template_entries.len();

    let changing_template_x: Vec<i8> = changing_template_entries
        .iter()
        .map(|&k| template[k].x as i8)
        .collect();
    let changing_template_y: Vec<i8> = changing_template_entries
        .iter()
        .map(|&k| template[k].y as i8)
        .collect();
    let changing_template_bit: Vec<u16> = changing_template_entries
        .iter()
        .map(|&k| 1u16 << (template_length - 1 - k))
        .collect();

    // Get the safe bounding box edges from the width, height, minX, maxX, minY
    let sbb_left = -min_x;
    let sbb_top = -min_y;
    let sbb_right = width as i32 - max_x;

    let pseudo_pixel_context = REUSED_CONTEXTS[template_index];
    let mut bitmap = Vec::with_capacity(height);
    let mut row = Rc::new(RefCell::new(vec![0u8; width]));

    let decoder = &mut decoding_context.decoder;
    let contexts = decoding_context.context_cache.get_contexts("GB");

    let mut ltp = 0u8;
    let mut context_label = 0u32;

    for i in 0..height {
        if prediction {
            let sltp = decoder.read_bit(contexts, pseudo_pixel_context as usize);
            ltp ^= sltp;

            if ltp != 0 {
                bitmap.push(row.clone()); // duplicate previous row
                continue;
            }
        }

        let old_data = row.borrow().clone();
        row = Rc::new(RefCell::new(old_data));
        bitmap.push(row.clone());

        for j in 0..width {
            if useskip && skip.unwrap()[i][j] != 0 {
                row.borrow_mut()[j] = 0;
                continue;
            }

            // Are we in the middle of a scanline, so we can reuse contextLabel bits?
            if (j as i32) >= sbb_left && (j as i32) < sbb_right && (i as i32) >= sbb_top {
                // If yes, we can just shift the bits that are reusable and only
                // fetch the remaining ones.
                context_label = (context_label << 1) & reuse_mask;
                for k in 0..changing_entries_length {
                    // println!("k_if: {k}, {context_label}");
                    let i0 = (i as i32 + changing_template_y[k] as i32) as usize;
                    let j0 = (j as i32 + changing_template_x[k] as i32) as usize;
                    let bit = bitmap[i0].borrow()[j0];
                    if bit != 0 {
                        context_label |= changing_template_bit[k] as u32;
                    }
                }
            } else {
                // compute the contextLabel from scratch
                context_label = 0;
                let mut shift = template_length - 1;
                for k in 0..template_length {
                    // println!("k_else: {k}, {context_label}");
                    let j0 = j as i32 + template_x[k] as i32;
                    if j0 >= 0 && j0 < width as i32 {
                        let i0 = i as i32 + template_y[k] as i32;
                        if i0 >= 0 {
                            let bit = bitmap[i0 as usize].borrow()[j0 as usize];
                            if bit != 0 {
                                context_label |= (bit as u32) << shift;
                            }
                        }
                    }

                    if shift > 0 {
                        shift -= 1;
                    }
                }
            }

            let pixel = decoder.read_bit(contexts, context_label as usize);
            row.borrow_mut()[j] = pixel;
        }
    }

    Ok(bitmap.into_iter().map(|i| i.borrow().clone()).collect())
}
