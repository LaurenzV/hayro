//! Performing the inverse discrete wavelet transform, as specified in Annex F.

use crate::codestream::WaveletTransform;
use crate::packet::{SubBand, SubbandType};
use crate::tile::IntRect;
use std::iter;

const PADDING_SHIFT: usize = 4;

pub(crate) fn apply(subbands: &[Vec<SubBand>], transform: WaveletTransform) -> Vec<f32> {
    let mut ll_subband = subbands[0][0].clone();

    for subbands in &subbands[1..] {
        let [hl, lh, hh] = subbands.as_slice() else {
            unreachable!()
        };

        let new_rect = {
            let x0 = ll_subband.rect.x0;
            let x1 = x0 + ll_subband.rect.width() + hl.rect.width();
            let y0 = ll_subband.rect.y0;
            let y1 = y0 + ll_subband.rect.height() + lh.rect.height();

            IntRect::from_xywh(x0, y0, x1, y1)
        };

        ll_subband = _2d_sr(&ll_subband, &hl, &lh, &hh, new_rect, transform);
    }

    eprintln!("{:?}", &ll_subband.coefficients);

    ll_subband.coefficients
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

    let temp_rect = IntRect::from_ltrb(0, 0, rect.width(), rect.height());

    hor_sr(&mut coefficients, temp_rect, &transform);
    ver_sr(&mut coefficients, temp_rect, &transform);

    SubBand {
        subband_type: SubbandType::LowLow,
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
    for subband in [ll, hl, lh, hh] {
        let u_max = match subband.subband_type {
            SubbandType::LowLow | SubbandType::LowHigh => rect.width().div_ceil(2),
            SubbandType::HighLow | SubbandType::HighHigh => rect.width() / 2,
        };

        let v_max = match subband.subband_type {
            SubbandType::LowLow | SubbandType::HighLow => rect.height().div_ceil(2),
            SubbandType::LowHigh | SubbandType::HighHigh => rect.height() / 2,
        };

        for v in 0..v_max {
            for u in 0..u_max {
                let (x, y) = match subband.subband_type {
                    SubbandType::LowLow => (2 * u, 2 * v),
                    SubbandType::LowHigh => (2 * u, 2 * v + 1),
                    SubbandType::HighLow => (2 * u + 1, 2 * v),
                    SubbandType::HighHigh => (2 * u + 1, 2 * v + 1),
                };

                coefficients[(y * rect.width() + x) as usize] =
                    subband.coefficients[(v * subband.rect.width() + u) as usize];
            }
        }
    }

    coefficients
}

/// The HOR_SR procedure from F.3.4.
fn hor_sr(a: &mut [f32], rect: IntRect, transform: &WaveletTransform) {
    // Add a padding of 8 to account for the _1d_extr procedure.
    let mut buf = vec![0.0; rect.width() as usize + 10];

    let shift = PADDING_SHIFT + if rect.x0 % 2 != 0 { 1 } else { 0 };

    for v in rect.y0..rect.y1 {
        buf.clear();
        // Add left padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        // Extract row into buffer.
        buf.extend_from_slice(&a[(rect.width() * v) as usize..][..rect.width() as usize]);

        // Add right padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        _1d_sr(&mut buf, shift, shift + rect.width() as usize, transform);

        // Put values back into original array.
        a[(rect.width() * v) as usize..][..rect.width() as usize]
            .copy_from_slice(&buf[shift..][..rect.width() as usize]);
    }
}

/// The VER_SR procedure from F.3.5.
fn ver_sr(a: &mut [f32], rect: IntRect, transform: &WaveletTransform) {
    // Add a padding of 8 to account for the _1d_extr procedure.
    let mut buf = vec![0.0; rect.height() as usize + 10];

    let shift = PADDING_SHIFT + if rect.y0 % 2 != 0 { 1 } else { 0 };

    for u in rect.x0..rect.x1 {
        buf.clear();
        // Add left padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        // Extract column into buffer.
        for y in rect.y0..rect.y1 {
            buf.push(a[(u + rect.width() * y) as usize]);
        }

        // Add right padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        _1d_sr(&mut buf, shift, shift + rect.height() as usize, transform);

        // Put values back into original array.
        for (idx, y) in (rect.y0..rect.y1).enumerate() {
            a[(u + rect.width() * y) as usize] = buf[shift + idx]
        }
    }
}

/// The 1D_SR procedure from F.3.6
fn _1d_sr(y: &mut [f32], i0: usize, i1: usize, transform: &WaveletTransform) {
    if i0 == i1 - 1 {
        if i0 % 2 != 0 {
            y[i0] = y[i0] / 2.0;
        }

        return;
    }

    _1d_extr(y, i0, i1, transform);

    match transform {
        WaveletTransform::Irreversible97 => unimplemented!(),
        WaveletTransform::Reversible53 => _1d_filter_53r(y, i0, i1),
    }
}

/// The 1D FILTER 5-3R procedure from F.3.8.1.
fn _1d_filter_53r(y: &mut [f32], i0: usize, i1: usize) {
    // (F-5)
    for n in i0 / 2..(i1 / 2) + 1 {
        let base_idx = 2 * n;
        y[base_idx] = y[base_idx] - ((y[base_idx - 1] + y[base_idx + 1] + 2.0) / 4.0).floor();
    }

    // (F-6)
    for n in i0 / 2..(i1 / 2) {
        let base_idx = 2 * n + 1;
        y[base_idx] = y[base_idx] + ((y[base_idx - 1] + y[base_idx + 1]) / 2.0).floor();
    }
}

/// The 1D_EXTR procedure.
fn _1d_extr(y: &mut [f32], i0: usize, i1: usize, transform: &WaveletTransform) {
    let i_left = match transform {
        WaveletTransform::Reversible53 => {
            if i0 % 2 == 0 {
                1
            } else {
                2
            }
        }
        WaveletTransform::Irreversible97 => {
            if i0 % 2 == 0 {
                3
            } else {
                4
            }
        }
    };

    let i_right = match transform {
        WaveletTransform::Reversible53 => {
            if i1 % 2 == 0 {
                2
            } else {
                1
            }
        }
        WaveletTransform::Irreversible97 => {
            if i1 % 2 == 0 {
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
