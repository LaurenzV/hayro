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
