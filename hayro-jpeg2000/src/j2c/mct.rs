//! The irreversible multi-component transformation, as specified in
//! Annex G.2 and G.3.

use super::codestream::{Header, WaveletTransform};
use super::decode::TileDecodeContext;
use super::simd::{Level, Simd, dispatch, f32x8};

/// Apply the inverse multi-component transform, as specified in G.2 and G.3.
pub(crate) fn apply_inverse(
    tile_ctx: &mut TileDecodeContext<'_>,
    header: &Header<'_>,
) -> Result<(), &'static str> {
    if tile_ctx.channel_data.len() < 3 {
        return if header.strict {
            Err("tried to apply MCT to image with less than 3 components")
        } else {
            Ok(())
        };
    }

    let (s, _) = tile_ctx.channel_data.split_at_mut(3);
    let [s0, s1, s2] = s else { unreachable!() };
    let s0 = &mut s0.container;
    let s1 = &mut s1.container;
    let s2 = &mut s2.container;

    let transform = tile_ctx.tile.component_infos[0].wavelet_transform();

    if transform != tile_ctx.tile.component_infos[1].wavelet_transform()
        || tile_ctx.tile.component_infos[1].wavelet_transform()
            != tile_ctx.tile.component_infos[2].wavelet_transform()
    {
        return Err("tried to apply MCT to image with different wavelet transforms per component");
    }

    let len = s0.len();

    if len != s1.len() || s1.len() != s2.len() {
        return Err("tried to apply MCT to image with different number of samples per component");
    }

    let new_len = len.next_multiple_of(8);
    s0.resize(new_len, 0.0);
    s1.resize(new_len, 0.0);
    s2.resize(new_len, 0.0);

    apply_inner(transform, s0, s1, s2);

    s0.truncate(len);
    s1.truncate(len);
    s2.truncate(len);

    Ok(())
}

fn apply_inner(transform: WaveletTransform, s0: &mut [f32], s1: &mut [f32], s2: &mut [f32]) {
    dispatch!(Level::new(), simd => apply_inner_impl(simd, transform, s0, s1, s2));
}

#[inline(always)]
fn apply_inner_impl<S: Simd>(
    simd: S,
    transform: WaveletTransform,
    s0: &mut [f32],
    s1: &mut [f32],
    s2: &mut [f32],
) {
    match transform {
        // Irreversible MCT, specified in G.3.
        WaveletTransform::Irreversible97 => {
            for ((y0, y1), y2) in s0
                .chunks_exact_mut(8)
                .zip(s1.chunks_exact_mut(8))
                .zip(s2.chunks_exact_mut(8))
            {
                let y_0 = f32x8::from_slice(simd, y0);
                let y_1 = f32x8::from_slice(simd, y1);
                let y_2 = f32x8::from_slice(simd, y2);

                let i0 = y_2.madd(1.402, y_0);
                let i1 = y_2.madd(-0.71414, y_1.madd(-0.34413, y_0));
                let i2 = y_1.madd(1.772, y_0);

                i0.store(y0);
                i1.store(y1);
                i2.store(y2);
            }
        }
        // Reversible MCT, specified in G.2.
        WaveletTransform::Reversible53 => {
            for ((y0, y1), y2) in s0
                .chunks_exact_mut(8)
                .zip(s1.chunks_exact_mut(8))
                .zip(s2.chunks_exact_mut(8))
            {
                let y_0 = f32x8::from_slice(simd, y0);
                let y_1 = f32x8::from_slice(simd, y1);
                let y_2 = f32x8::from_slice(simd, y2);

                let i1 = y_0 - ((y_2 + y_1) * 0.25).floor();
                let i0 = y_2 + i1;
                let i2 = y_1 + i1;

                i0.store(y0);
                i1.store(y1);
                i2.store(y2);
            }
        }
    }
}
