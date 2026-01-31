//! Gray-scale image decoding procedure (Annex C).

use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::Bitmap;
use crate::decode::generic::{ContextGatherer, decode_bitmap_mmr};
use crate::decode::{AdaptiveTemplatePixel, Template};
use crate::error::Result;

/// Input parameters to the gray-scale image decoding procedure (Table C.1).
#[derive(Debug, Clone)]
pub(crate) struct GrayScaleParams<'a> {
    /// `GSMMR` - Specifies whether MMR is used.
    pub(crate) use_mmr: bool,
    /// `GSBPP` - The number of bits per gray-scale value.
    pub(crate) bits_per_pixel: u32,
    /// `GSW` - The width of the gray-scale image.
    pub(crate) width: u32,
    /// `GSH` - The height of the gray-scale image.
    pub(crate) height: u32,
    /// `GSTEMPLATE` - The template used to code the gray-scale bitplanes.
    pub(crate) template: Template,
    /// `GSKIP` - A mask indicating which values should be skipped.
    /// None if `GSUSESKIP` = 0.
    pub(crate) skip_mask: Option<&'a [u32]>,
}

/// The gray-scale image decoding procedure (Annex C, C.5).
///
/// Returns `GSVALS`: the decoded gray-scale image array, GSW × GSH pixels.
#[inline(always)]
pub(crate) fn decode_gray_scale_image(
    data: &[u8],
    params: &GrayScaleParams<'_>,
) -> Result<Vec<u32>> {
    // Table C.1: "GSMMR specifies whether MMR is used."
    if params.use_mmr {
        decode_mmr(data, params)
    } else {
        decode_arithmetic(data, params)
    }
}

/// The gray-scale image decoding procedure using MMR (Annex C, C.5).
///
/// Table C.4: "MMR = GSMMR"
fn decode_mmr(data: &[u8], params: &GrayScaleParams<'_>) -> Result<Vec<u32>> {
    // `GSW` - The width of the gray-scale image.
    let width = params.width;
    // `GSH` - The height of the gray-scale image.
    let height = params.height;
    // `GSBPP` - The number of bits per gray-scale value.
    let bits_per_pixel = params.bits_per_pixel;
    let stride = width.div_ceil(32);

    let mut offset = 0;
    decode_bitplanes(width, height, stride, bits_per_pixel, |_| {
        // Table C.4: "GBW = GSW, GBH = GSH"
        let mut bitplane = Bitmap::new(width, height);
        offset += decode_bitmap_mmr(&mut bitplane, &data[offset..])?;
        Ok(bitplane.data)
    })
}

/// The gray-scale image decoding procedure using arithmetic coding (Annex C, C.5).
///
/// Table C.4: "GBTEMPLATE = GSTEMPLATE, TPGDON = 0, USESKIP = GSUSESKIP, SKIP = GSKIP"
fn decode_arithmetic(data: &[u8], params: &GrayScaleParams<'_>) -> Result<Vec<u32>> {
    // `GSW` - The width of the gray-scale image.
    let width = params.width;
    // `GSH` - The height of the gray-scale image.
    let height = params.height;
    // `GSBPP` - The number of bits per gray-scale value.
    let bits_per_pixel = params.bits_per_pixel;
    let stride = width.div_ceil(32);
    // `GSKIP` - The skip mask (if GSUSESKIP = 1).
    let skip_mask = params.skip_mask;
    // `GSTEMPLATE` - The template used for bitplane decoding.
    let template = params.template;

    // Table C.4: Adaptive template pixel positions.
    let at_pixels: Vec<AdaptiveTemplatePixel> = match template {
        Template::Template0 => vec![
            AdaptiveTemplatePixel { x: 3, y: -1 },
            AdaptiveTemplatePixel { x: -3, y: -1 },
            AdaptiveTemplatePixel { x: 2, y: -2 },
            AdaptiveTemplatePixel { x: -2, y: -2 },
        ],
        Template::Template1 => vec![AdaptiveTemplatePixel { x: 3, y: -1 }],
        Template::Template2 | Template::Template3 => {
            vec![AdaptiveTemplatePixel { x: 2, y: -1 }]
        }
    };

    let mut decoder = ArithmeticDecoder::new(data);
    let mut contexts = vec![Context::default(); 1 << template.context_bits()];

    decode_bitplanes(width, height, stride, bits_per_pixel, |_| {
        // Table C.4: "GBW = GSW, GBH = GSH, TPGDON = 0"
        let mut bitplane = Bitmap::new(width, height);
        let mut gatherer = ContextGatherer::new(width, height, template, &at_pixels);

        for y in 0..height {
            gatherer.start_row(&bitplane, y);
            for x in 0..width {
                // Table C.4: "USESKIP = GSUSESKIP, SKIP = GSKIP"
                if let Some(mask) = skip_mask {
                    let word_idx = (y * stride + x / 32) as usize;
                    let bit_pos = 31 - (x % 32);
                    if (mask[word_idx] >> bit_pos) & 1 != 0 {
                        // Still need to update the context.
                        let _ = gatherer.gather(&bitplane, x);
                        gatherer.update_current_row(x, false);
                        continue;
                    }
                }

                let context = gatherer.gather(&bitplane, x);
                let pixel = decoder.decode(&mut contexts[context as usize]);
                let value = pixel != 0;

                bitplane.set_pixel(x, y, value);
                gatherer.update_current_row(x, value);
            }
        }

        Ok(bitplane.data)
    })
}

/// The bitplane decoding and gray value computation procedure (C.5).
///
/// The closure `decode_next` is called for each bitplane, receiving the bitplane
/// index (`GSBPP`-1 down to 0) and returning the decoded bitplane data in packed format.
fn decode_bitplanes<F>(
    width: u32,
    height: u32,
    stride: u32,
    bits_per_pixel: u32,
    mut decode_next: F,
) -> Result<Vec<u32>>
where
    F: FnMut(u32) -> Result<Vec<u32>>,
{
    let size = (width * height) as usize;
    // `GSVALS` - The decoded gray-scale image array.
    let mut values = vec![0_u32; size];

    // C.5 step 1: "Decode GSPLANES[GSBPP - 1]"
    // `GSPLANES` - Bitplanes of the gray-scale image.
    let mut prev_plane = decode_next(bits_per_pixel - 1)?;

    // The first (MSB) bitplane contributes directly to the gray values.
    // Extract bits from packed format.
    for y in 0..height {
        for x in 0..width {
            let word_idx = (y * stride + x / 32) as usize;
            let bit_pos = 31 - (x % 32);
            if (prev_plane[word_idx] >> bit_pos) & 1 != 0 {
                let i = (y * width + x) as usize;
                values[i] |= 1 << (bits_per_pixel - 1);
            }
        }
    }

    // C.5 step 2: "Set J = GSBPP - 2."
    // C.5 step 3: "While J >= 0:"
    // `J` - Bitplane counter.
    for j in (0..bits_per_pixel - 1).rev() {
        // C.5 step 3a: "Decode GSPLANES[J]"
        let mut plane = decode_next(j)?;

        // This step applies gray coding.
        // C.5 step 3b: "GSPLANES[J][x, y] = GSPLANES[J + 1][x, y] XOR GSPLANES[J][x, y]"
        // With packed format, we can XOR whole words.
        for i in 0..plane.len() {
            plane[i] ^= prev_plane[i];
        }

        // C.5 step 4: "GSVALS[x, y] = sum(J=0 to GSBPP-1) GSPLANES[J][x, y] * 2^J"
        for y in 0..height {
            for x in 0..width {
                let word_idx = (y * stride + x / 32) as usize;
                let bit_pos = 31 - (x % 32);
                if (plane[word_idx] >> bit_pos) & 1 != 0 {
                    let i = (y * width + x) as usize;
                    values[i] |= 1 << j;
                }
            }
        }

        // C.5 step 3c: "Set J = J - 1."
        prev_plane = plane;
    }

    Ok(values)
}
