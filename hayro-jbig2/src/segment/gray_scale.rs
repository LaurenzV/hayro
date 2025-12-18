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
    if params.use_mmr {
        decode_gray_scale_mmr(data, params)
    } else {
        decode_gray_scale_arith(data, params)
    }
}

/// Decode gray-scale image using MMR encoding.
fn decode_gray_scale_mmr(
    data: &[u8],
    params: &GrayScaleParams<'_>,
) -> Result<Vec<u32>, &'static str> {
    let width = params.width;
    let height = params.height;
    let bits_per_pixel = params.bits_per_pixel;

    // GSPLANES: Array of bitplanes (Table C.3)
    let mut bitplanes: Vec<DecodedRegion> = Vec::with_capacity(bits_per_pixel as usize);

    let mut offset = 0;

    // "1) Decode GSPLANES[GSBPP – 1]" (C.5)
    let mut bitplane = DecodedRegion::new(width, height);
    decode_bitmap_mmr(&mut bitplane, &data[offset..])?;
    offset += estimate_mmr_size(&data[offset..]);
    bitplanes.push(bitplane);

    // "2) Set J = GSBPP – 2." (C.5)
    // "3) While J ≥ 0:" (C.5)
    for _ in (0..bits_per_pixel.saturating_sub(1)).rev() {
        // "a) Decode GSPLANES[J]" (C.5)
        let mut bitplane = DecodedRegion::new(width, height);
        decode_bitmap_mmr(&mut bitplane, &data[offset..])?;
        offset += estimate_mmr_size(&data[offset..]);

        // "b) GSPLANES[J][x, y] = GSPLANES[J + 1][x, y] XOR GSPLANES[J][x, y]" (C.5)
        let prev_plane = bitplanes.last().unwrap();
        for i in 0..bitplane.data.len() {
            bitplane.data[i] = prev_plane.data[i] ^ bitplane.data[i];
        }

        bitplanes.push(bitplane);
    }

    // "4) GSVALS[x, y] = Σ(J=0 to GSBPP-1) GSPLANES[J][x, y] × 2^J" (C.5)
    compute_gray_values(&bitplanes, width, height, bits_per_pixel)
}

/// Decode gray-scale image using arithmetic encoding.
fn decode_gray_scale_arith(
    data: &[u8],
    params: &GrayScaleParams<'_>,
) -> Result<Vec<u32>, &'static str> {
    let width = params.width;
    let height = params.height;
    let bits_per_pixel = params.bits_per_pixel;

    // Build AT pixels according to Table C.4.
    let at_pixels = build_at_pixels(params.template);
    let gb_template = params.template.to_gb_template();

    let num_context_bits = match gb_template {
        GbTemplate::Template0 => 16,
        GbTemplate::Template1 => 13,
        GbTemplate::Template2 | GbTemplate::Template3 => 10,
    };

    // GSPLANES: Array of bitplanes (Table C.3)
    let mut bitplanes: Vec<DecodedRegion> = Vec::with_capacity(bits_per_pixel as usize);

    // All bitplanes share the same arithmetic decoder and context statistics.
    let mut decoder = ArithmeticDecoder::new(data);
    let mut contexts = vec![ArithmeticDecoderContext::default(); 1 << num_context_bits];

    // "1) Decode GSPLANES[GSBPP – 1]" (C.5)
    let bitplane = decode_bitplane_arith(
        &mut decoder,
        &mut contexts,
        width,
        height,
        gb_template,
        &at_pixels,
        params.skip_mask,
    )?;
    bitplanes.push(bitplane);

    // "2) Set J = GSBPP – 2." (C.5)
    // "3) While J ≥ 0:" (C.5)
    for _ in (0..bits_per_pixel.saturating_sub(1)).rev() {
        // "a) Decode GSPLANES[J]" (C.5)
        let mut bitplane = decode_bitplane_arith(
            &mut decoder,
            &mut contexts,
            width,
            height,
            gb_template,
            &at_pixels,
            params.skip_mask,
        )?;

        // "b) GSPLANES[J][x, y] = GSPLANES[J + 1][x, y] XOR GSPLANES[J][x, y]" (C.5)
        let prev_plane = bitplanes.last().unwrap();
        for i in 0..bitplane.data.len() {
            bitplane.data[i] = prev_plane.data[i] ^ bitplane.data[i];
        }

        bitplanes.push(bitplane);
    }

    // "4) GSVALS[x, y] = Σ(J=0 to GSBPP-1) GSPLANES[J][x, y] × 2^J" (C.5)
    compute_gray_values(&bitplanes, width, height, bits_per_pixel)
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
) -> Result<DecodedRegion, &'static str> {
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

    Ok(bitplane)
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

/// Compute gray values from bitplanes (C.5 step 4).
///
/// GSVALS[x, y] = Σ(J=0 to GSBPP-1) GSPLANES[J][x, y] × 2^J
fn compute_gray_values(
    bitplanes: &[DecodedRegion],
    width: u32,
    height: u32,
    bits_per_pixel: u32,
) -> Result<Vec<u32>, &'static str> {
    let size = (width * height) as usize;
    let mut values = vec![0u32; size];

    // bitplanes[0] is GSPLANES[GSBPP-1] (MSB, decoded first)
    // bitplanes[GSBPP-1] is GSPLANES[0] (LSB, decoded last)
    // After XOR, each bitplane contains the actual binary value for that bit position.

    for i in 0..size {
        let mut value = 0u32;

        for (plane_idx, plane) in bitplanes.iter().enumerate() {
            let bit_position = bits_per_pixel - 1 - plane_idx as u32;

            if plane.data[i] {
                value |= 1u32 << bit_position;
            }
        }

        values[i] = value;
    }

    Ok(values)
}

/// Estimate MMR data size for one bitplane by looking for EOFB marker.
fn estimate_mmr_size(data: &[u8]) -> usize {
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == 0x00 && data[i + 1] == 0x00 {
            let end = (i + 4).min(data.len());
            return end;
        }
    }

    data.len()
}
