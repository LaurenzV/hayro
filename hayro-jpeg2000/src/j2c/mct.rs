use super::ComponentData;
use super::codestream::{Header, WaveletTransform};
use crate::cancel::{CHUNK, Cancel};
use crate::error::{ColorError, Result, bail, err};
use crate::math::{Level, Simd, dispatch, f32x8};

pub(crate) fn apply_inverse(
    components: &mut [ComponentData],
    component_infos: &[super::codestream::ComponentInfo],
    header: &Header,
    cancel: Cancel<'_>,
) -> Result<()> {
    if components.len() < 3 {
        return if header.strict {
            err!(ColorError::Mct)
        } else {
            Ok(())
        };
    }

    let (s, _) = components.split_at_mut(3);
    let [s0, s1, s2] = s else { unreachable!() };

    let transform = component_infos[0].wavelet_transform();

    if transform != component_infos[1].wavelet_transform()
        || component_infos[1].wavelet_transform() != component_infos[2].wavelet_transform()
    {
        bail!(ColorError::Mct);
    }

    if s0.container.len() != s1.container.len() || s1.container.len() != s2.container.len() {
        bail!(ColorError::Mct);
    }

    for ((c0, c1), c2) in s0
        .container
        .chunks_mut(CHUNK)
        .zip(s1.container.chunks_mut(CHUNK))
        .zip(s2.container.chunks_mut(CHUNK))
    {
        cancel.check()?;

        apply_inner(transform, c0, c1, c2);
    }

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
        WaveletTransform::Irreversible97 => {
            for ((y0, y1), y2) in s0
                .as_chunks_mut::<8>()
                .0
                .iter_mut()
                .zip(s1.as_chunks_mut::<8>().0)
                .zip(s2.as_chunks_mut::<8>().0)
            {
                let y_0 = f32x8::from_slice(simd, y0);
                let y_1 = f32x8::from_slice(simd, y1);
                let y_2 = f32x8::from_slice(simd, y2);

                let i0 = y_2.mul_add(f32x8::splat(simd, 1.402), y_0);
                let i1 = y_2.mul_add(
                    f32x8::splat(simd, -0.71414),
                    y_1.mul_add(f32x8::splat(simd, -0.34413), y_0),
                );
                let i2 = y_1.mul_add(f32x8::splat(simd, 1.772), y_0);

                i0.store(y0);
                i1.store(y1);
                i2.store(y2);
            }
        }
        WaveletTransform::Reversible53 => {
            for ((y0, y1), y2) in s0
                .as_chunks_mut::<8>()
                .0
                .iter_mut()
                .zip(s1.as_chunks_mut::<8>().0)
                .zip(s2.as_chunks_mut::<8>().0)
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
