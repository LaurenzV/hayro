use crate::filter::jbig2::{Bitmap, DecodingContext};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn decode_bitmap_template0(
    width: usize,
    height: usize,
    decoding_context: &mut DecodingContext,
) -> Bitmap {
    let contexts = decoding_context.context_cache.get_contexts("GB");
    let decoder = &mut decoding_context.decoder;
    let mut bitmap = Vec::with_capacity(height);

    // ...ooooo....
    // ..ooooooo... Context template for current pixel (X)
    // .ooooX...... (concatenate values of 'o'-pixels to get contextLabel)
    const OLD_PIXEL_MASK: u32 = 0x7bf7; // 01111 0111111 0111

    for i in 0..height {
        let mut row = Rc::new(RefCell::new(vec![0u8; width]));
        bitmap.push(row.clone());
        let row1 = if i < 1 {
            row.clone()
        } else {
            bitmap[i - 1].clone()
        };
        let row2 = if i < 2 {
            row.clone()
        } else {
            bitmap[i - 2].clone()
        };

        // At the beginning of each row:
        // Fill contextLabel with pixels that are above/right of (X)
        let mut context_label = (row2.borrow()[0] as u32) << 13
            | (row2.borrow()[1] as u32) << 12
            | (row2.borrow()[1] as u32) << 11
            | (row1.borrow()[0] as u32) << 7
            | (row1.borrow()[1] as u32) << 6
            | (row1.borrow()[2] as u32) << 5
            | (row1.borrow()[3] as u32) << 4;

        for j in 0..width {
            let pixel = decoder.read_bit(contexts, context_label as usize);
            row.borrow_mut()[j] = pixel;

            // At each pixel: Clear contextLabel pixels that are shifted
            // out of the context, then add new ones.

            context_label = ((context_label & OLD_PIXEL_MASK) << 1)
                | {
                    if j + 3 < width {
                        (row2.borrow()[j + 3] as u32) << 11
                    } else {
                        0
                    }
                }
                | {
                    if j + 4 < width {
                        (row1.borrow()[j + 4] as u32) << 4
                    } else {
                        0
                    }
                }
                | pixel as u32;
        }
    }

    bitmap.iter().map(|i| i.borrow().clone()).collect()
}
