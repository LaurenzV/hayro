use crate::filter::jbig2::tables::{REFINEMENT_REUSED_CONTEXTS, REFINEMENT_TEMPLATES};
use crate::filter::jbig2::{Bitmap, DecodingContext, Jbig2Error, TemplatePixel};
use std::cell::RefCell;
use std::rc::Rc;

// 6.3.2 Generic Refinement Region Decoding Procedure
pub(crate) fn decode_refinement(
    width: usize,
    height: usize,
    template_index: usize,
    reference_bitmap: &Bitmap,
    offset_x: i32,
    offset_y: i32,
    prediction: bool,
    at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
) -> Result<Bitmap, Jbig2Error> {
    let mut coding_template: Vec<[i32; 2]> = REFINEMENT_TEMPLATES[template_index].coding.to_vec();
    if template_index == 0 {
        coding_template.push([at[0].x, at[0].y]);
    }
    let coding_template_length = coding_template.len();

    let mut coding_template_x = vec![0i32; coding_template_length];
    let mut coding_template_y = vec![0i32; coding_template_length];
    for k in 0..coding_template_length {
        coding_template_x[k] = coding_template[k][0];
        coding_template_y[k] = coding_template[k][1];
    }

    let mut reference_template: Vec<[i32; 2]> =
        REFINEMENT_TEMPLATES[template_index].reference.to_vec();
    if template_index == 0 {
        reference_template.push([at[1].x, at[1].y]);
    }
    let reference_template_length = reference_template.len();

    let mut reference_template_x = vec![0i32; reference_template_length];
    let mut reference_template_y = vec![0i32; reference_template_length];
    for k in 0..reference_template_length {
        reference_template_x[k] = reference_template[k][0];
        reference_template_y[k] = reference_template[k][1];
    }

    let reference_width = reference_bitmap[0].len();
    let reference_height = reference_bitmap.len();

    let pseudo_pixel_context = REFINEMENT_REUSED_CONTEXTS[template_index];
    let mut bitmap = vec![];

    let decoder = &mut decoding_context.decoder;
    let contexts = decoding_context.context_cache.get_contexts("GR");

    let mut ltp = 0u8;

    for i in 0..height {
        if prediction {
            let sltp = decoder.read_bit(contexts, pseudo_pixel_context as usize);
            ltp ^= sltp;
            if ltp != 0 {
                return Err(Jbig2Error::new("prediction is not supported"));
            }
        }

        let mut row = Rc::new(RefCell::new(vec![0u8; width]));
        bitmap.push(row.clone());

        for j in 0..width {
            let mut context_label = 0u32;

            for k in 0..coding_template_length {
                let i0 = i as i32 + coding_template_y[k];
                let j0 = j as i32 + coding_template_x[k];

                if i0 < 0 || j0 < 0 || j0 >= width as i32 {
                    context_label <<= 1; // out of bound pixel
                } else {
                    context_label =
                        (context_label << 1) | (bitmap[i0 as usize].borrow()[j0 as usize] as u32);
                }
            }

            for k in 0..reference_template_length {
                let i0 = i as i32 + reference_template_y[k] - offset_y;
                let j0 = j as i32 + reference_template_x[k] - offset_x;

                if i0 < 0 || i0 >= reference_height as i32 || j0 < 0 || j0 >= reference_width as i32
                {
                    context_label <<= 1; // out of bound pixel
                } else {
                    context_label =
                        (context_label << 1) | (reference_bitmap[i0 as usize][j0 as usize] as u32);
                }
            }

            let pixel = decoder.read_bit(contexts, context_label as usize);
            row.borrow_mut()[j] = pixel;
        }
        bitmap.push(row);
    }

    Ok(bitmap.into_iter().map(|i| i.borrow().clone()).collect())
}
