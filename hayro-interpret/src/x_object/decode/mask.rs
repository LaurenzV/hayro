use super::{DecodeContext, decode_context, fix_image_length, unpack_samples};
use crate::LumaData;
use crate::function::interpolate;
use crate::x_object::image::{ImageKind, ImageXObject};
use std::sync::LazyLock;

static BILEVEL_MASK_LUT: LazyLock<Box<[u64; 256]>> = LazyLock::new(|| {
    let mut lut = Box::new([0; 256]);
    for (byte, expanded) in lut.iter_mut().enumerate() {
        let mut pixels = [0; 8];
        for (bit, pixel) in pixels.iter_mut().enumerate() {
            *pixel = if byte & (0x80 >> bit) != 0 { 255 } else { 0 };
        }
        *expanded = u64::from_ne_bytes(pixels);
    }
    lut
});

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

    let bilevel_fast_path = ctx.bits_per_component == 1
        && ctx.color_space.num_components() == 1
        && (ctx.decode_arr.as_slice() == default_decode.as_slice()
            || ctx.decode_arr.as_slice() == inverted_default.as_slice());
    let fast_path = ctx.bits_per_component == 8
        && (ctx.decode_arr.as_slice() == default_decode.as_slice()
            || ctx.decode_arr.as_slice() == inverted_default.as_slice());

    let mut data = if bilevel_fast_path {
        decode_bilevel_mask_data(
            &ctx,
            invert ^ (ctx.decode_arr.as_slice() == inverted_default.as_slice()),
        )
    } else if fast_path {
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

fn decode_bilevel_mask_data(ctx: &DecodeContext<'_>, should_invert: bool) -> Vec<u8> {
    let width = ctx.width as usize;
    let height = ctx.height as usize;
    let row_bytes = width.div_ceil(8);
    let data = ctx.decoded.data.as_ref();

    let xor_mask = if should_invert { u64::MAX } else { 0 };
    let full_bytes = width / 8;
    let tail_bits = width % 8;
    let lut = &**BILEVEL_MASK_LUT;
    let mut out = Vec::with_capacity(width * height);
    // To avoid repeatedly calling `extend`, which turns out to be very
    // expensive.
    let mut buffer = [[0; 8]; 32];

    for row in data.chunks(row_bytes).take(height) {
        for chunk in row[..row.len().min(full_bytes)].chunks(buffer.len()) {
            for (expanded, &byte) in buffer.iter_mut().zip(chunk) {
                *expanded = (lut[byte as usize] ^ xor_mask).to_ne_bytes();
            }
            out.extend_from_slice(buffer[..chunk.len()].as_flattened());
        }

        if tail_bits != 0 && row.len() > full_bytes {
            let expanded = (lut[row[full_bytes] as usize] ^ xor_mask).to_ne_bytes();
            out.extend_from_slice(&expanded[..tail_bits]);
        }
    }
    out
}
