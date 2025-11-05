//! Performing the inverse discrete wavelet transform, as specified in Annex F.

use crate::codestream::WaveletTransform;
use crate::packet::{Decomposition, SubBand, SubbandType};
use crate::tile::IntRect;
use std::iter;

/// The amount of padding to apply to a single scanline to make filtering at
/// the boundary possible.
const PADDING_SHIFT: usize = 4;

pub(crate) fn apply(
    // The lower LL subband for resolution level 0.
    ll_subband: &SubBand,
    // All decomposition level that make up the tile.
    decompositions: &[Decomposition],
    tile_rect: IntRect,
    transform: WaveletTransform,
) -> Vec<f32> {
    let mut ll_subband = ll_subband.clone();

    for decomposition in decompositions {
        let ll_rect = decomposition.sub_bands[0].ll_rect;

        ll_subband = filter_2d(&ll_subband, &decomposition, ll_rect, transform);
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

/// The 2D_INTERLEAVE procedure described in F.3.3.
fn filter_2d(
    ll: &SubBand,
    decomposition: &Decomposition,
    rect: IntRect,
    transform: WaveletTransform,
) -> SubBand<'static> {
    let mut coefficients = interleave_samples(ll, decomposition, rect);

    filter_horizontal(&mut coefficients, rect, &transform);
    filter_vertical(&mut coefficients, rect, &transform);

    SubBand {
        sub_band_type: SubbandType::LowLow,
        ll_rect: rect,
        rect,
        precincts: vec![],
        coefficients,
    }
}

/// The 2D_INTERLEAVE procedure described in F.3.3.
fn interleave_samples(ll: &SubBand, decomposition: &Decomposition, rect: IntRect) -> Vec<f32> {
    let mut coefficients = vec![0.0; (rect.width() * rect.height()) as usize];
    let IntRect {
        x0: u0,
        x1: u1,
        y0: v0,
        y1: v1,
    } = rect;

    for subband in [
        ll,
        &decomposition.sub_bands[0],
        &decomposition.sub_bands[1],
        &decomposition.sub_bands[2],
    ] {
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
fn filter_horizontal(scan_line: &mut [f32], rect: IntRect, transform: &WaveletTransform) {
    // Add a padding of 8 to account for the _1d_extr procedure.
    let mut buf = vec![0.0; rect.width() as usize + 10];

    // There's some subtlety going on here. The extension procedure defined in
    // the spec is based on the start and end values i0 and i1 which are
    // dependent on the rectangle of the subband we are currently processing.
    // The problem is that if we use the values as is, if we for example had the
    // i0/i1 values larger than 1000, we would have to allocate a buffer of
    // length at least 1000, even though the width/height of the rectangle is
    // much less. Looking at the equations more closely, it becomes apparent
    // that the real value of i0/i1 is not relevant, and the behavior of
    // subsequent operations only really depends on whether val % 2 == 0 or
    // not. Therefore, we shift the values of i0 and i1 such that the property
    // still remains the same, but the values themselves are much smaller.

    let shift = PADDING_SHIFT + if !rect.x0.is_multiple_of(2) { 1 } else { 0 };

    for v in 0..rect.height() {
        buf.clear();
        // Add left padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        let start_idx = rect.width() as usize * v as usize;

        // Extract row into buffer.
        buf.extend_from_slice(&scan_line[start_idx..][..rect.width() as usize]);

        // Add right padding for 1D_EXTR procedure.
        buf.extend(iter::repeat_n(0.0, shift));

        filter_single_row(&mut buf, shift, shift + rect.width() as usize, transform);

        // Put values back into original array.
        scan_line[start_idx..][..rect.width() as usize]
            .copy_from_slice(&buf[shift..][..rect.width() as usize]);
    }
}

/// The VER_SR procedure from F.3.5.
fn filter_vertical(a: &mut [f32], rect: IntRect, transform: &WaveletTransform) {
    // Add a padding of 8 to account for the 1D_EXTR procedure.
    // TODO: Reuse buffer.
    let mut buf = vec![0.0; rect.height() as usize + 10];

    // See the comment in `filter_horizontal`.
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

        filter_single_row(&mut buf, shift, shift + rect.height() as usize, transform);

        // Put values back into original array.
        for (idx, y) in (0..rect.height()).enumerate() {
            a[(u + rect.width() * y) as usize] = buf[shift + idx]
        }
    }
}

/// The 1D_SR procedure from F.3.6.
fn filter_single_row(scanline: &mut [f32], start: usize, end: usize, transform: &WaveletTransform) {
    if start == end - 1 {
        if !start.is_multiple_of(2) {
            scanline[start] /= 2.0;
        }

        return;
    }

    extend_signal(scanline, start, end, transform);

    match transform {
        WaveletTransform::Reversible53 => reversible_filter_53r(scanline, start, end),
        WaveletTransform::Irreversible97 => irreversible_filter_97i(scanline, start, end),
    }
}

/// The 1D FILTER 5-3R procedure from F.3.8.1.
fn reversible_filter_53r(scanline: &mut [f32], start: usize, end: usize) {
    // Equation (F-5).
    for n in start / 2..(end / 2) + 1 {
        let base_idx = 2 * n;
        scanline[base_idx] -=
            ((scanline[base_idx - 1] + scanline[base_idx + 1] + 2.0) / 4.0).floor();
    }

    // Equation (F-6).
    for n in start / 2..(end / 2) {
        let base_idx = 2 * n + 1;
        scanline[base_idx] += ((scanline[base_idx - 1] + scanline[base_idx + 1]) / 2.0).floor();
    }
}

/// The 1D Filter 9-7I procedure from F.3.8.2.
fn irreversible_filter_97i(scanline: &mut [f32], start: usize, end: usize) {
    // Table F.4.
    const ALPHA: f32 = -1.586_134_3;
    const BETA: f32 = -0.052_980_117;
    const GAMMA: f32 = 0.882_911_1;
    const DELTA: f32 = 0.443_506_87;
    const KAPPA: f32 = 1.230_174_1;

    // Step 1.
    for i in (start / 2 - 1)..(end / 2 + 2) {
        scanline[2 * i] *= KAPPA;
    }

    // Step 2.
    for i in (start / 2 - 2)..(end / 2 + 2) {
        scanline[2 * i + 1] *= (1.0 / KAPPA);
    }

    // Step 3.
    for i in (start / 2 - 1)..(end / 2 + 2) {
        scanline[2 * i] -= DELTA * (scanline[2 * i - 1] + scanline[2 * i + 1]);
    }

    // Step 4.
    for i in (start / 2 - 1)..(end / 2 + 1) {
        scanline[2 * i + 1] -= GAMMA * (scanline[2 * i] + scanline[2 * i + 2]);
    }

    // Step 5.
    for i in (start / 2)..(end / 2 + 1) {
        scanline[2 * i] -= BETA * (scanline[2 * i - 1] + scanline[2 * i + 1]);
    }

    // Step 6.
    for i in (start / 2)..(end / 2) {
        scanline[2 * i + 1] -= ALPHA * (scanline[2 * i] + scanline[2 * i + 2]);
    }
}

/// The 1D_EXTR procedure, defined in F.3.7.
fn extend_signal(scanline: &mut [f32], start: usize, end: usize, transform: &WaveletTransform) {
    let i_left = match transform {
        WaveletTransform::Reversible53 => {
            if start.is_multiple_of(2) {
                1
            } else {
                2
            }
        }
        WaveletTransform::Irreversible97 => {
            if start.is_multiple_of(2) {
                3
            } else {
                4
            }
        }
    };

    let i_right = match transform {
        WaveletTransform::Reversible53 => {
            if end.is_multiple_of(2) {
                2
            } else {
                1
            }
        }
        WaveletTransform::Irreversible97 => {
            if end.is_multiple_of(2) {
                4
            } else {
                3
            }
        }
    };

    for i in (start - i_left)..start {
        scanline[i] = scanline[periodic_symmetric_extension(i, start, end)];
    }

    for i in end..(end + i_right) {
        scanline[i] = scanline[periodic_symmetric_extension(i, start, end)];
    }
}

/// Perform the periodic symmetric extension, specified in Equation (F-4).
fn periodic_symmetric_extension(idx: usize, start: usize, end: usize) -> usize {
    let span = 2 * (end as i32 - start as i32 - 1);
    let offset = (idx as i32 - start as i32).rem_euclid(span);
    (start as i32 + offset.min(span - offset)) as usize
}

#[cfg(test)]
mod tests {
    use crate::codestream::WaveletTransform;

    #[test]
    fn pse() {
        assert_eq!(super::periodic_symmetric_extension(0, 3, 6), 4);
        assert_eq!(super::periodic_symmetric_extension(1, 3, 6), 5);
        assert_eq!(super::periodic_symmetric_extension(2, 3, 6), 4);
        assert_eq!(super::periodic_symmetric_extension(3, 3, 6), 3);
        assert_eq!(super::periodic_symmetric_extension(4, 3, 6), 4);
        assert_eq!(super::periodic_symmetric_extension(5, 3, 6), 5);
        assert_eq!(super::periodic_symmetric_extension(6, 3, 6), 4);
        assert_eq!(super::periodic_symmetric_extension(7, 3, 6), 3);
        assert_eq!(super::periodic_symmetric_extension(8, 3, 6), 4);
        assert_eq!(super::periodic_symmetric_extension(9, 3, 6), 5);
    }

    #[test]
    fn extend_1d() {
        let mut data = [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0];
        super::extend_signal(&mut data, 3, 9, &WaveletTransform::Reversible53);

        assert_eq!(
            data,
            [0.0, 3.0, 2.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 0.0]
        );
    }
}
