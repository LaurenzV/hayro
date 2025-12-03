//! The irreversible multi-component transformation, as specified in
//! Annex G.2 and G.3.

use crate::codestream::Header;
use crate::decode::TileDecodeContext;

/// Apply the inverse multi-component transform, as specified in G.2 and G.3.
pub(crate) fn apply_inverse(
    tile_ctx: &mut TileDecodeContext,
    header: &Header,
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

    simd::apply_inner(transform, s0, s1, s2);

    s0.truncate(len);
    s1.truncate(len);
    s2.truncate(len);

    Ok(())
}

#[cfg(not(feature = "simd"))]
mod simd {
    use crate::codestream::WaveletTransform;

    pub(super) fn apply_inner(
        transform: WaveletTransform,
        s0: &mut [f32],
        s1: &mut [f32],
        s2: &mut [f32],
    ) {
        match transform {
            WaveletTransform::Irreversible97 => {
                for ((y0, y1), y2) in s0
                    .chunks_exact_mut(8)
                    .zip(s1.chunks_exact_mut(8))
                    .zip(s2.chunks_exact_mut(8))
                {
                    for lane in 0..8 {
                        let y_0 = y0[lane];
                        let y_1 = y1[lane];
                        let y_2 = y2[lane];

                        let i0 = y_2.mul_add(1.402, y_0);
                        let i1 = y_2.mul_add(-0.71414, y_1.mul_add(-0.34413, y_0));
                        let i2 = y_1.mul_add(1.772, y_0);

                        y0[lane] = i0;
                        y1[lane] = i1;
                        y2[lane] = i2;
                    }
                }
            }
            WaveletTransform::Reversible53 => {
                for ((y0, y1), y2) in s0
                    .chunks_exact_mut(8)
                    .zip(s1.chunks_exact_mut(8))
                    .zip(s2.chunks_exact_mut(8))
                {
                    for lane in 0..8 {
                        let y_0 = y0[lane];
                        let y_1 = y1[lane];
                        let y_2 = y2[lane];

                        let i1 = y_0 - ((y_2 + y_1) * 0.25).floor();
                        let i0 = y_2 + i1;
                        let i2 = y_1 + i1;

                        y0[lane] = i0;
                        y1[lane] = i1;
                        y2[lane] = i2;
                    }
                }
            }
        }
    }
}

#[cfg(feature = "simd")]
mod simd {
    use crate::codestream::WaveletTransform;
    use fearless_simd::*;

    pub(super) fn apply_inner(
        transform: WaveletTransform,
        s0: &mut [f32],
        s1: &mut [f32],
        s2: &mut [f32],
    ) {
        dispatch!(Level::new(), simd => apply_inner_simd(simd, transform, s0, s1, s2));
    }

    #[inline(always)]
    fn apply_inner_simd<S: Simd>(
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

                    y0.copy_from_slice(&i0.val);
                    y1.copy_from_slice(&i1.val);
                    y2.copy_from_slice(&i2.val);
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

                    y0.copy_from_slice(&i0.val);
                    y1.copy_from_slice(&i1.val);
                    y2.copy_from_slice(&i2.val);
                }
            }
        }
    }
}
