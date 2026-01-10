//! Gray-scale image decoding procedure (Annex C).

use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::error::Result;
use crate::region::generic::{
    AdaptiveTemplatePixel, GbTemplate, decode_bitmap_mmr, gather_context_with_at,
};

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
    pub(crate) template: GbTemplate,
    /// `GSKIP` - A mask indicating which values should be skipped.
    /// GSW pixels wide, GSH pixels high. None if `GSUSESKIP` = 0.
    pub(crate) skip_mask: Option<&'a [bool]>,
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
    let size = (width * height) as usize;

    let mut offset = 0;
    decode_bitplanes(bits_per_pixel, size, |_| {
        // Table C.4: "GBW = GSW, GBH = GSH"
        let mut bitplane = DecodedRegion::new(width, height);
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
    let size = (width * height) as usize;
    // `GSKIP` - The skip mask (if GSUSESKIP = 1).
    let skip_mask = params.skip_mask;
    // `GSTEMPLATE` - The template used for bitplane decoding.
    let template = params.template;

    // Table C.4: Adaptive template pixel positions.
    let at_pixels: Vec<AdaptiveTemplatePixel> = match template {
        GbTemplate::Template0 => vec![
            AdaptiveTemplatePixel { x: 3, y: -1 },
            AdaptiveTemplatePixel { x: -3, y: -1 },
            AdaptiveTemplatePixel { x: 2, y: -2 },
            AdaptiveTemplatePixel { x: -2, y: -2 },
        ],
        GbTemplate::Template1 => vec![AdaptiveTemplatePixel { x: 3, y: -1 }],
        GbTemplate::Template2 | GbTemplate::Template3 => {
            vec![AdaptiveTemplatePixel { x: 2, y: -1 }]
        }
    };

    let mut decoder = ArithmeticDecoder::new(data);
    let mut contexts = vec![Context::default(); 1 << template.context_bits()];

    decode_bitplanes(bits_per_pixel, size, |_| {
        // Table C.4: "GBW = GSW, GBH = GSH, TPGDON = 0"
        let mut bitplane = DecodedRegion::new(width, height);

        for y in 0..height {
            for x in 0..width {
                // Table C.4: "USESKIP = GSUSESKIP, SKIP = GSKIP"
                if let Some(mask) = skip_mask {
                    let idx = (y * width + x) as usize;
                    if mask[idx] {
                        continue;
                    }
                }

                let context = gather_context_with_at(&bitplane, x, y, template, &at_pixels);
                let pixel = decoder.decode(&mut contexts[context as usize]);

                bitplane.set_pixel(x, y, pixel != 0);
            }
        }

        Ok(bitplane.data)
    })
}

/// The bitplane decoding and gray value computation procedure (C.5).
///
/// The closure `decode_next` is called for each bitplane, receiving the bitplane
/// index (`GSBPP`-1 down to 0) and returning the decoded bitplane data.
fn decode_bitplanes<F>(bits_per_pixel: u32, size: usize, mut decode_next: F) -> Result<Vec<u32>>
where
    F: FnMut(u32) -> Result<Vec<bool>>,
{
    // `GSVALS` - The decoded gray-scale image array.
    let mut values = vec![0_u32; size];

    // C.5 step 1: "Decode GSPLANES[GSBPP - 1]"
    // `GSPLANES` - Bitplanes of the gray-scale image.
    let mut prev_plane = decode_next(bits_per_pixel - 1)?;

    // The first (MSB) bitplane contributes directly to the gray values.
    for (i, &bit) in prev_plane.iter().enumerate() {
        if bit {
            values[i] |= 1 << (bits_per_pixel - 1);
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
        for i in 0..size {
            plane[i] ^= prev_plane[i];
        }

        // C.5 step 4: "GSVALS[x, y] = sum(J=0 to GSBPP-1) GSPLANES[J][x, y] * 2^J"
        for (i, &bit) in plane.iter().enumerate() {
            if bit {
                values[i] |= 1 << j;
            }
        }

        // C.5 step 3c: "Set J = J - 1."
        prev_plane = plane;
    }

    Ok(values)
}
