//! Performing the inverse discrete wavelet transform, as specified in Annex F.

use crate::codestream::WaveletTransform;
use crate::packet::{Decomposition, SubBand, SubbandType};
use crate::tile::IntRect;
use std::iter;

const PADDING_SHIFT: usize = 4;

pub(crate) fn apply(
    ll_subband: &SubBand,
    decompositions: &[Decomposition],
    tile_rect: IntRect,
    transform: WaveletTransform,
) -> Vec<f32> {
    let mut ll_subband = ll_subband.clone();

    for decomposition in decompositions {
        let ll_rect = decomposition.sub_bands[0].ll_rect;

        ll_subband = _2d_sr(
            &ll_subband,
            &decomposition.sub_bands[0],
            &decomposition.sub_bands[1],
            &decomposition.sub_bands[2],
            ll_rect,
            transform,
        );
    }

    let mut trimmed_coefficients = Vec::with_capacity(ll_subband.coefficients.len());

    let skip_y = tile_rect.y0 - ll_subband.rect.y0;
    let take_y = tile_rect.height();
    let skip_x = tile_rect.x0 - ll_subband.rect.x0;
    let take_x = tile_rect.width();

    for row in ll_subband
        .coefficients
        .chunks_exact(ll_subband.rect.width() as usize)
        .skip(skip_y as usize)
        .take(take_y as usize)
    {
        trimmed_coefficients.extend(&row[skip_x as usize..][..take_x as usize])
    }

    trimmed_coefficients
}

fn _2d_sr(
    ll: &SubBand,
    hl: &SubBand,
    lh: &SubBand,
    hh: &SubBand,
    rect: IntRect,
    transform: WaveletTransform,
) -> SubBand<'static> {
    let mut coefficients = _2d_interleave(ll, hl, lh, hh, rect);

    hor_sr(&mut coefficients, rect, &transform);
    ver_sr(&mut coefficients, rect, &transform);

    SubBand {
        sub_band_type: SubbandType::LowLow,
        ll_rect: rect,
        rect,
        precincts: vec![],
        coefficients,
    }
}

fn _2d_interleave(
    ll: &SubBand,
    hl: &SubBand,
    lh: &SubBand,
    hh: &SubBand,
    rect: IntRect,
) -> Vec<f32> {
    let mut coefficients = vec![0.0; (rect.width() * rect.height()) as usize];
    let IntRect {
        x0: u0,
        x1: u1,
        y0: v0,
        y1: v1,
    } = rect;

    for subband in [ll, hl, lh, hh] {
        let (u_min, u_max) = match subband.sub_band_type {
            SubbandType::LowLow | SubbandType::LowHigh => (u0.div_ceil(2), u1.div_ceil(2)),
            SubbandType::HighLow | SubbandType::HighHigh => (u0 / 2, u1 / 2),
        };

        let (v_min, v_max) = match subband.sub_band_type {
            SubbandType::LowLow | SubbandType::HighLow => (v0.div_ceil(2), v1.div_ceil(2)),
            SubbandType::LowHigh | SubbandType::HighHigh => (v0 / 2, v1 / 2),
        };

        for v_b in v_min..v_max {
            for u_b in u_min..u_max {
                let (x, y) = match subband.sub_band_type {
                    SubbandType::LowLow => (2 * u_b, 2 * v_b),
                    SubbandType::LowHigh => (2 * u_b, 2 * v_b + 1),
                    SubbandType::HighLow => (2 * u_b + 1, 2 * v_b),
                    SubbandType::HighHigh => (2 * u_b + 1, 2 * v_b + 1),
                };

                coefficients[((y - v0) * rect.width() + (x - u0)) as usize] = subband.coefficients
                    [((v_b - v_min) * subband.rect.width() + (u_b - u_min)) as usize];
            }
        }
    }

    coefficients
}

/// The HOR_SR procedure from F.3.4.
fn hor_sr(a: &mut [f32], rect: IntRect, transform: &WaveletTransform) {
    // Add a padding of 8 to account for the _1d_extr procedure.
    let mut buf = vec![0.0; rect.width() as usize + 10];

    let shift = PADDING_SHIFT + if !rect.x0.is_multiple_of(2) { 1 } else { 0 };

    for v in 0..rect.height() {
        buf.clear();
        // Add left padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        let start_idx = rect.width() as usize * v as usize;

        // Extract row into buffer.
        buf.extend_from_slice(&a[start_idx..][..rect.width() as usize]);

        // Add right padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        _1d_sr(&mut buf, shift, shift + rect.width() as usize, transform);

        // Put values back into original array.
        a[start_idx..][..rect.width() as usize]
            .copy_from_slice(&buf[shift..][..rect.width() as usize]);
    }
}

/// The VER_SR procedure from F.3.5.
fn ver_sr(a: &mut [f32], rect: IntRect, transform: &WaveletTransform) {
    // Add a padding of 8 to account for the _1d_extr procedure.
    let mut buf = vec![0.0; rect.height() as usize + 10];

    let shift = PADDING_SHIFT + if !rect.y0.is_multiple_of(2) { 1 } else { 0 };

    for u in 0..rect.width() {
        buf.clear();
        // Add left padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        // Extract column into buffer.
        for y in 0..rect.height() {
            buf.push(a[(u + rect.width() * y) as usize]);
        }

        // Add right padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        _1d_sr(&mut buf, shift, shift + rect.height() as usize, transform);

        // Put values back into original array.
        for (idx, y) in (0..rect.height()).enumerate() {
            a[(u + rect.width() * y) as usize] = buf[shift + idx]
        }
    }
}

/// The 1D_SR procedure from F.3.6
fn _1d_sr(y: &mut [f32], i0: usize, i1: usize, transform: &WaveletTransform) {
    if i0 == i1 - 1 {
        if !i0.is_multiple_of(2) {
            y[i0] /= 2.0;
        }

        return;
    }

    _1d_extr(y, i0, i1, transform);

    match transform {
        WaveletTransform::Reversible53 => _1d_filter_53r(y, i0, i1),
        WaveletTransform::Irreversible97 => _1d_filter_97(y, i0, i1),
    }
}

/// The 1D FILTER 5-3R procedure from F.3.8.1.
fn _1d_filter_53r(y: &mut [f32], i0: usize, i1: usize) {
    // (F-5)
    for n in i0 / 2..(i1 / 2) + 1 {
        let base_idx = 2 * n;
        y[base_idx] -= ((y[base_idx - 1] + y[base_idx + 1] + 2.0) / 4.0).floor();
    }

    // (F-6)
    for n in i0 / 2..(i1 / 2) {
        let base_idx = 2 * n + 1;
        y[base_idx] += ((y[base_idx - 1] + y[base_idx + 1]) / 2.0).floor();
    }
}

/// The 1D Filter 9-7I procedure from F.3.8.2
fn _1d_filter_97(y: &mut [f32], i0: usize, i1: usize) {
    // Table F.4.
    const ALPHA: f32 = -1.586_134_3;
    const BETA: f32 = -0.052_980_117;
    const GAMMA: f32 = 0.882_911_1;
    const DELTA: f32 = 0.443_506_87;
    const KAPPA: f32 = 1.230_174_1;

    // Step 1
    for i in (i0 / 2 - 1)..(i1 / 2 + 2) {
        y[2 * i] *= KAPPA;
    }

    // Step 2
    for i in (i0 / 2 - 2)..(i1 / 2 + 2) {
        y[2 * i + 1] *= (1.0 / KAPPA);
    }

    // Step 3
    for i in (i0 / 2 - 1)..(i1 / 2 + 2) {
        y[2 * i] -= DELTA * (y[2 * i - 1] + y[2 * i + 1]);
    }

    // Step 4
    for i in (i0 / 2 - 1)..(i1 / 2 + 1) {
        y[2 * i + 1] -= GAMMA * (y[2 * i] + y[2 * i + 2]);
    }

    // Step 5
    for i in (i0 / 2)..(i1 / 2 + 1) {
        y[2 * i] -= BETA * (y[2 * i - 1] + y[2 * i + 1]);
    }

    // Step 6
    for i in (i0 / 2)..(i1 / 2) {
        y[2 * i + 1] -= ALPHA * (y[2 * i] + y[2 * i + 2]);
    }
}

/// The 1D_EXTR procedure.
fn _1d_extr(y: &mut [f32], i0: usize, i1: usize, transform: &WaveletTransform) {
    let i_left = match transform {
        WaveletTransform::Reversible53 => {
            if i0.is_multiple_of(2) {
                1
            } else {
                2
            }
        }
        WaveletTransform::Irreversible97 => {
            if i0.is_multiple_of(2) {
                3
            } else {
                4
            }
        }
    };

    let i_right = match transform {
        WaveletTransform::Reversible53 => {
            if i1.is_multiple_of(2) {
                2
            } else {
                1
            }
        }
        WaveletTransform::Irreversible97 => {
            if i1.is_multiple_of(2) {
                4
            } else {
                3
            }
        }
    };

    for i in (i0 - i_left)..i0 {
        y[i] = y[pse(i, i0, i1)];
    }

    for i in i1..(i1 + i_right) {
        y[i] = y[pse(i, i0, i1)];
    }
}

/// Equation (F-4).
fn pse(i: usize, i0: usize, i1: usize) -> usize {
    let span = 2 * (i1 as i32 - i0 as i32 - 1);
    let m = (i as i32 - i0 as i32).rem_euclid(span);
    (i0 as i32 + m.min(span - m)) as usize
}

#[cfg(test)]
mod tests {
    use crate::codestream::WaveletTransform;

    #[test]
    fn pse() {
        assert_eq!(super::pse(0, 3, 6), 4);
        assert_eq!(super::pse(1, 3, 6), 5);
        assert_eq!(super::pse(2, 3, 6), 4);
        assert_eq!(super::pse(3, 3, 6), 3);
        assert_eq!(super::pse(4, 3, 6), 4);
        assert_eq!(super::pse(5, 3, 6), 5);
        assert_eq!(super::pse(6, 3, 6), 4);
        assert_eq!(super::pse(7, 3, 6), 3);
        assert_eq!(super::pse(8, 3, 6), 4);
        assert_eq!(super::pse(9, 3, 6), 5);
    }

    #[test]
    fn extend_1d() {
        let mut data = [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0];
        super::_1d_extr(&mut data, 3, 9, &WaveletTransform::Reversible53);

        assert_eq!(
            data,
            [0.0, 3.0, 2.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 0.0]
        );
    }
}
