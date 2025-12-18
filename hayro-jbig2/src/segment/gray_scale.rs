//! Gray-scale image decoding procedure (Annex C).

use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::DecodedRegion;
use crate::segment::generic_region::{
    AdaptiveTemplatePixel, GbTemplate, decode_bitmap_mmr, gather_context_with_at,
};

/// Template used for gray-scale image decoding (Table C.1: GSTEMPLATE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GsTemplate {
    Template0 = 0,
    Template1 = 1,
    Template2 = 2,
    Template3 = 3,
}

impl GsTemplate {
    /// Convert to GbTemplate for generic region decoding (Table C.4: GBTEMPLATE = GSTEMPLATE).
    fn to_gb_template(self) -> GbTemplate {
        match self {
            GsTemplate::Template0 => GbTemplate::Template0,
            GsTemplate::Template1 => GbTemplate::Template1,
            GsTemplate::Template2 => GbTemplate::Template2,
            GsTemplate::Template3 => GbTemplate::Template3,
        }
    }
}

/// Input parameters to the gray-scale image decoding procedure (Table C.1).
#[derive(Debug, Clone)]
pub(crate) struct GrayScaleParams<'a> {
    /// Whether MMR encoding is used (GSMMR).
    pub use_mmr: bool,
    /// The number of bits per gray-scale value (GSBPP).
    pub bits_per_pixel: u32,
    /// The width of the gray-scale image (GSW).
    pub width: u32,
    /// The height of the gray-scale image (GSH).
    pub height: u32,
    /// The template used to code the gray-scale bitplanes (GSTEMPLATE).
    pub template: GsTemplate,
    /// A mask indicating which values should be skipped (GSKIP).
    /// Width × height pixels. None if skipping is disabled (GSUSESKIP = 0).
    pub skip_mask: Option<&'a [bool]>,
}

/// Decode a gray-scale image (Annex C).
///
/// Returns GSVALS: the decoded gray-scale image array, width × height pixels.
pub(crate) fn decode_gray_scale_image(
    data: &[u8],
    params: &GrayScaleParams<'_>,
) -> Result<Vec<u32>, &'static str> {
    let width = params.width;
    let height = params.height;
    let bits_per_pixel = params.bits_per_pixel;
    let size = (width * height) as usize;

    if params.use_mmr {
        let mut offset = 0;
        decode_bitplanes(bits_per_pixel, size, |_| {
            let mut bitplane = DecodedRegion::new(width, height);
            offset += decode_bitmap_mmr(&mut bitplane, &data[offset..])?;
            Ok(bitplane.data)
        })
    } else {
        let at_pixels = build_at_pixels(params.template);
        let gb_template = params.template.to_gb_template();
        let num_context_bits = match gb_template {
            GbTemplate::Template0 => 16,
            GbTemplate::Template1 => 13,
            GbTemplate::Template2 | GbTemplate::Template3 => 10,
        };

        // All bitplanes share the same arithmetic decoder and context statistics.
        let mut decoder = ArithmeticDecoder::new(data);
        let mut contexts = vec![ArithmeticDecoderContext::default(); 1 << num_context_bits];

        decode_bitplanes(bits_per_pixel, size, |_| {
            decode_bitplane_arith(
                &mut decoder,
                &mut contexts,
                width,
                height,
                gb_template,
                &at_pixels,
                params.skip_mask,
            )
        })
    }
}

/// Decode bitplanes and compute gray values (C.5).
///
/// The closure `decode_next` is called for each bitplane, receiving the bitplane
/// index (GSBPP-1 down to 0) and returning the decoded bitplane data.
fn decode_bitplanes<F>(
    bits_per_pixel: u32,
    size: usize,
    mut decode_next: F,
) -> Result<Vec<u32>, &'static str>
where
    F: FnMut(u32) -> Result<Vec<bool>, &'static str>,
{
    let mut values = vec![0u32; size];

    // "1) Decode GSPLANES[GSBPP – 1]" (C.5)
    let mut prev_plane = decode_next(bits_per_pixel - 1)?;

    // The first (MSB) bitplane contributes directly to the gray values.
    for (i, &bit) in prev_plane.iter().enumerate() {
        if bit {
            values[i] |= 1 << (bits_per_pixel - 1);
        }
    }

    // "2) Set J = GSBPP – 2." (C.5)
    // "3) While J ≥ 0:" (C.5)
    for j in (0..bits_per_pixel.saturating_sub(1)).rev() {
        // "a) Decode GSPLANES[J]" (C.5)
        let mut plane = decode_next(j)?;

        // "b) GSPLANES[J][x, y] = GSPLANES[J + 1][x, y] XOR GSPLANES[J][x, y]" (C.5)
        for i in 0..size {
            plane[i] ^= prev_plane[i];
        }

        // Accumulate into gray values.
        // "4) GSVALS[x, y] = Σ(J=0 to GSBPP-1) GSPLANES[J][x, y] × 2^J" (C.5)
        for (i, &bit) in plane.iter().enumerate() {
            if bit {
                values[i] |= 1 << j;
            }
        }

        prev_plane = plane;
    }

    Ok(values)
}

/// Decode a single bitplane using arithmetic coding.
///
/// Implements the generic region decoding procedure with Table C.4 parameters:
/// TPGDON = 0, USESKIP = GSUSESKIP, SKIP = GSKIP.
fn decode_bitplane_arith(
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [ArithmeticDecoderContext],
    width: u32,
    height: u32,
    gb_template: GbTemplate,
    at_pixels: &[AdaptiveTemplatePixel],
    skip_mask: Option<&[bool]>,
) -> Result<Vec<bool>, &'static str> {
    let mut bitplane = DecodedRegion::new(width, height);

    // TPGDON = 0: no typical prediction, decode every pixel.
    for y in 0..height {
        for x in 0..width {
            // USESKIP/SKIP (Table C.4): skip if mask indicates this pixel should be skipped.
            if let Some(mask) = skip_mask {
                let idx = (y * width + x) as usize;
                if mask[idx] {
                    continue; // Leave as 0
                }
            }

            let context = gather_context_with_at(&bitplane, x, y, gb_template, at_pixels);
            let pixel = decoder.decode(&mut contexts[context as usize]);

            bitplane.set_pixel(x, y, pixel != 0);
        }
    }

    Ok(bitplane.data)
}

/// Build adaptive template pixels for gray-scale image decoding (Table C.4).
///
/// GBATX1 = 3 if GSTEMPLATE ≤ 1; 2 if GSTEMPLATE ≥ 2
/// GBATY1 = -1
/// GBATX2 = -3, GBATY2 = -1
/// GBATX3 = 2, GBATY3 = -2
/// GBATX4 = -2, GBATY4 = -2
fn build_at_pixels(template: GsTemplate) -> Vec<AdaptiveTemplatePixel> {
    match template {
        GsTemplate::Template0 => {
            vec![
                AdaptiveTemplatePixel { x: 3, y: -1 },
                AdaptiveTemplatePixel { x: -3, y: -1 },
                AdaptiveTemplatePixel { x: 2, y: -2 },
                AdaptiveTemplatePixel { x: -2, y: -2 },
            ]
        }
        GsTemplate::Template1 => {
            vec![AdaptiveTemplatePixel { x: 3, y: -1 }]
        }
        GsTemplate::Template2 | GsTemplate::Template3 => {
            vec![AdaptiveTemplatePixel { x: 2, y: -1 }]
        }
    }
}
