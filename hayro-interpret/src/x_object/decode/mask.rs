use super::{DecodeContext, decode_context, fix_image_length, unpack_samples};
use crate::LumaData;
use crate::function::interpolate;
use crate::x_object::image::{ImageKind, ImageXObject};

pub(crate) struct DecodedMask {
    pub(crate) luma: LumaData,
}

pub(crate) fn decode_mask(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodedMask> {
    let ctx = decode_context(obj, target_dimension)?;
    let width = ctx.width;
    let scale_factors = ctx.scale_factors;

    // Note: The semantics between "normal" soft masks (i.e. masks defined in
    // the graphics state or via `Mask`/`SMask` are inverted compared to
    // stencil masks (defined via `ImageMask`). The former match the semantics
    // of normal alpha images, where 0 stands for invisible and MAX stands for
    // fully opaque. For stencil masks, it's the other way around: 1 means the
    // paint is visible, while 0 means it's invisible.
    let invert = obj.kind == ImageKind::StencilMask;
    let (data, height) = decode_mask_data(ctx, invert)?;

    Some(DecodedMask {
        luma: LumaData {
            data,
            width,
            height,
            interpolate: obj.interpolate,
            scale_factors,
        },
    })
}

fn decode_mask_data(mut ctx: DecodeContext<'_>, invert: bool) -> Option<(Vec<u8>, u32)> {
    let default_decode = ctx
        .color_space
        .default_decode_arr(ctx.bits_per_component as f32);
    let inverted_default = ctx
        .color_space
        .inverted_default_decode_arr(ctx.bits_per_component as f32);

    // 1-bit masks (the common case for stencil masks) only ever produce 0 or 255,
    // so expand them with a byte-wide lookup table instead of going through the
    // general per-sample `BitReader` + f32 interpolation path below, which is
    // several times slower for large masks (see #1319).
    if ctx.bits_per_component == 1
        && ctx.color_space.num_components() == 1
        && (ctx.decode_arr.as_slice() == default_decode.as_slice()
            || ctx.decode_arr.as_slice() == inverted_default.as_slice())
        && let Some(decoded) = decode_bilevel_mask_data(
            &ctx,
            invert ^ (ctx.decode_arr.as_slice() == inverted_default.as_slice()),
        )
    {
        return Some((decoded, ctx.height));
    }

    let fast_path = ctx.bits_per_component == 8
        && (ctx.decode_arr.as_slice() == default_decode.as_slice()
            || ctx.decode_arr.as_slice() == inverted_default.as_slice());

    let mut data = if fast_path {
        let mut decoded = ctx.decoded.data;
        let should_invert = invert ^ (ctx.decode_arr.as_slice() == inverted_default.as_slice());
        if should_invert {
            for byte in decoded.to_mut() {
                *byte = 255 - *byte;
            }
        }

        decoded.into_owned()
    } else {
        let num_components = ctx.color_space.num_components() as usize;
        let components = unpack_samples(
            &ctx.decoded.data,
            ctx.width,
            ctx.height,
            num_components,
            ctx.bits_per_component,
        )?;

        let source_max = 2.0_f32.powi(ctx.bits_per_component as i32) - 1.0;
        let mut decoded = Vec::with_capacity(components.len());

        for pixel in components.chunks(num_components) {
            for (component, (decode_min, decode_max)) in pixel.iter().zip(&ctx.decode_arr) {
                let value =
                    interpolate(*component as f32, 0.0, source_max, *decode_min, *decode_max);
                let value = if invert { 1.0 - value } else { value };
                decoded.push((value * 255.0 + 0.5) as u8);
            }
        }

        decoded
    };

    fix_image_length(
        &mut data,
        ctx.width,
        &mut ctx.height,
        0,
        ctx.color_space.num_components() as usize,
    )?;

    Some((data, ctx.height))
}

/// Expand a 1-bit mask (single component, default or inverted-default decode
/// array) into 0/255 luma bytes using a byte-wide lookup table that emits
/// 8 pixels per input byte.
///
/// Rows are byte-aligned, matching the `BitReader::align` call in the general
/// path. `should_invert` is the stencil inversion combined with an inverted
/// decode array. Returns `None` when the decoded data is too short for
/// `width x height`, leaving truncated files to the general path (which adapts
/// the height instead of padding whole rows).
fn decode_bilevel_mask_data(ctx: &DecodeContext<'_>, should_invert: bool) -> Option<Vec<u8>> {
    let width = ctx.width as usize;
    let height = ctx.height as usize;
    let row_bytes = width.div_ceil(8);
    let data = ctx.decoded.data.as_ref();
    if data.len() < row_bytes.checked_mul(height)? {
        return None;
    }

    let (zero, one) = if should_invert { (255, 0) } else { (0, 255) };
    let mut lut = [[0_u8; 8]; 256];
    for (byte, expanded) in lut.iter_mut().enumerate() {
        for (bit, out) in expanded.iter_mut().enumerate() {
            *out = if byte & (0x80 >> bit) != 0 { one } else { zero };
        }
    }

    let full_bytes = width / 8;
    let tail_bits = width % 8;
    let mut out = Vec::with_capacity(width * height);
    for row in data.chunks_exact(row_bytes).take(height) {
        for &byte in &row[..full_bytes] {
            out.extend_from_slice(&lut[byte as usize]);
        }
        if tail_bits != 0 {
            out.extend_from_slice(&lut[row[full_bytes] as usize][..tail_bits]);
        }
    }
    Some(out)
}
