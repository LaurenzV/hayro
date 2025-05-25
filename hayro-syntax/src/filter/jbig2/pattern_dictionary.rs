use crate::filter::jbig2::bitmap::decode_bitmap;
use crate::filter::jbig2::{Bitmap, DecodingContext, Jbig2Error, TemplatePixel, decode_mmr_bitmap};

// Pattern dictionary decoding - ported from decodePatternDictionary function
pub(crate) fn decode_pattern_dictionary(
    mmr: bool,
    pattern_width: usize,
    pattern_height: usize,
    max_pattern_index: usize,
    template: usize,
    decoding_context: &mut DecodingContext,
) -> Result<Vec<Bitmap>, Jbig2Error> {
    let mut at = Vec::new();
    if !mmr {
        at.push(TemplatePixel {
            x: -(pattern_width as i32),
            y: 0,
        });
        if template == 0 {
            at.push(TemplatePixel { x: -3, y: -1 });
            at.push(TemplatePixel { x: 2, y: -2 });
            at.push(TemplatePixel { x: -2, y: -2 });
        }
    }

    let collective_width = (max_pattern_index + 1) * pattern_width;
    let collective_bitmap = decode_bitmap(
        mmr,
        collective_width,
        pattern_height,
        template,
        false, // prediction
        None,  // skip
        &at,
        decoding_context,
    )?;

    // Divide collective bitmap into patterns.
    let mut patterns = Vec::new();
    for i in 0..=max_pattern_index {
        let mut pattern_bitmap = Vec::new();
        let x_min = pattern_width * i;
        let x_max = x_min + pattern_width;

        for y in 0..pattern_height {
            pattern_bitmap.push(collective_bitmap[y][x_min..x_max].to_vec());
        }
        patterns.push(pattern_bitmap);
    }

    Ok(patterns)
}
