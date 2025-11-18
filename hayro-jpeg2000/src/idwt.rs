//! Performing the inverse discrete wavelet transform, as specified in Annex F.

use crate::codestream::WaveletTransform;
use crate::decode::{Decomposition, SubBand, SubBandType};
use crate::rect::IntRect;

#[derive(Default, Copy, Clone)]
pub(crate) struct Padding {
    pub(crate) left: usize,
    pub(crate) top: usize,
    pub(crate) right: usize,
    pub(crate) bottom: usize,
}

impl Padding {
    fn new(left: usize, top: usize, right: usize, bottom: usize) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// The output from performing the IDWT operation.
pub(crate) struct IDWTOutput {
    pub(crate) coefficients: Vec<f32>,
    pub(crate) padding: Padding,
    /// The rect that the samples belong to. This will be equivalent
    /// to the rectangle that forms the smallest decomposition level. It does
    /// not have to be equivalent to the original size of the tile, as the
    /// sub-bands that form a tile aren't necessarily aligned to it. Therefore,
    /// the samples need to be trimmed to the tile rectangle afterward.
    pub(crate) rect: IntRect,
}

impl IDWTOutput {
    pub(crate) fn total_width(&self) -> u32 {
        self.padding.left as u32 + self.rect.width() + self.padding.right as u32
    }
}

/// Apply the inverse discrete wavelet transform (see Annex F). The output
/// will be transformed samples covering the rectangle of the smallest
/// decomposition level.
pub(crate) fn apply(
    // The LL sub-band for resolution level 0.
    ll_sub_band: &SubBand,
    // All decomposition level that make up the tile.
    decompositions: &[Decomposition],
    sub_bands: &[SubBand],
    transform: WaveletTransform,
) -> IDWTOutput {
    if decompositions.is_empty() {
        return IDWTOutput {
            coefficients: ll_sub_band.clone().coefficients,
            padding: Padding::default(),
            rect: ll_sub_band.rect,
        };
    }

    let mut temp_buf = vec![];

    let mut output = filter_2d(
        IDWTInput::from_sub_band(ll_sub_band),
        &decompositions[0],
        transform,
        &mut temp_buf,
        sub_bands,
    );

    for decomposition in decompositions.iter().skip(1) {
        output = filter_2d(
            IDWTInput::from_output(&output),
            decomposition,
            transform,
            &mut temp_buf,
            sub_bands,
        );
    }

    output
}

struct IDWTInput<'a> {
    coefficients: &'a [f32],
    padding: Padding,
    sub_band_type: SubBandType,
}

impl<'a> IDWTInput<'a> {
    fn from_sub_band(sub_band: &'a SubBand) -> IDWTInput<'a> {
        IDWTInput {
            coefficients: &sub_band.coefficients,
            padding: Padding::default(),
            sub_band_type: sub_band.sub_band_type,
        }
    }

    fn from_output(idwt_output: &'a IDWTOutput) -> IDWTInput<'a> {
        IDWTInput {
            coefficients: &idwt_output.coefficients,
            padding: idwt_output.padding,
            // The output from a previous iteration turns into the LL sub band
            // for the next iteration.
            sub_band_type: SubBandType::LowLow,
        }
    }
}

/// The 2D_INTERLEAVE procedure described in F.3.3.
fn filter_2d(
    // The LL sub band.
    input: IDWTInput,
    decomposition: &Decomposition,
    transform: WaveletTransform,
    temp_buf: &mut Vec<f32>,
    sub_bands: &[SubBand],
) -> IDWTOutput {
    let mut interleaved_samples = interleave_samples(input, decomposition, sub_bands, transform);

    if decomposition.rect.width() > 0 && decomposition.rect.height() > 0 {
        filter_horizontal(&mut interleaved_samples, decomposition.rect, transform);
        filter_vertical(
            &mut interleaved_samples,
            temp_buf,
            decomposition.rect,
            transform,
        );
    }

    IDWTOutput {
        coefficients: interleaved_samples.coefficients,
        rect: decomposition.rect,
        padding: interleaved_samples.padding,
    }
}

pub(crate) struct InterleavedSamples {
    pub(crate) coefficients: Vec<f32>,
    pub(crate) padding: Padding,
}

/// The 2D_INTERLEAVE procedure described in F.3.3.
fn interleave_samples(
    input: IDWTInput,
    decomposition: &Decomposition,
    sub_bands: &[SubBand],
    transform: WaveletTransform,
) -> InterleavedSamples {
    let new_padding = {
        let left_padding = left_extension(transform, decomposition.rect.x0 as usize) + 1;
        let top_padding = left_extension(transform, decomposition.rect.y0 as usize) + 1;
        let right_padding = right_extension(transform, decomposition.rect.x1 as usize);
        let bottom_padding = right_extension(transform, decomposition.rect.y1 as usize);

        Padding::new(left_padding, top_padding, right_padding, bottom_padding)
    };

    let total_width = decomposition.rect.width() as usize + new_padding.left + new_padding.right;
    let total_height = decomposition.rect.height() as usize + new_padding.top + new_padding.bottom;

    let mut interleaved = InterleavedSamples {
        coefficients: vec![0.0; total_width * total_height],
        padding: new_padding,
    };

    let IntRect {
        x0: u0,
        x1: u1,
        y0: v0,
        y1: v1,
    } = decomposition.rect;

    for idwt_input in [
        input,
        IDWTInput::from_sub_band(&sub_bands[decomposition.sub_bands[0]]),
        IDWTInput::from_sub_band(&sub_bands[decomposition.sub_bands[1]]),
        IDWTInput::from_sub_band(&sub_bands[decomposition.sub_bands[2]]),
    ] {
        let (u_min, u_max) = match idwt_input.sub_band_type {
            SubBandType::LowLow | SubBandType::LowHigh => (u0.div_ceil(2), u1.div_ceil(2)),
            SubBandType::HighLow | SubBandType::HighHigh => (u0 / 2, u1 / 2),
        };

        let (v_min, v_max) = match idwt_input.sub_band_type {
            SubBandType::LowLow | SubBandType::HighLow => (v0.div_ceil(2), v1.div_ceil(2)),
            SubBandType::LowHigh | SubBandType::HighHigh => (v0 / 2, v1 / 2),
        };

        let num_v = v_max - v_min;
        let num_u = u_max - u_min;

        let input_left_padding = idwt_input.padding.left;
        let input_right_padding = idwt_input.padding.right;
        let input_total_width = num_u + input_left_padding as u32 + input_right_padding as u32;

        if num_u == 0 || num_v == 0 {
            continue;
        }

        let (start_x, start_y) = match idwt_input.sub_band_type {
            SubBandType::LowLow => (2 * u_min, 2 * v_min),
            SubBandType::LowHigh => (2 * u_min, 2 * v_min + 1),
            SubBandType::HighLow => (2 * u_min + 1, 2 * v_min),
            SubBandType::HighHigh => (2 * u_min + 1, 2 * v_min + 1),
        };

        let coefficient_rows = interleaved
            .coefficients
            .chunks_exact_mut(total_width)
            .map(|s| &mut s[new_padding.left..][..decomposition.rect.width() as usize])
            .skip((start_y - v0) as usize + new_padding.top)
            .step_by(2);

        for (v_b, coefficient_row) in coefficient_rows.enumerate().take(num_v as usize) {
            // Hint compiler to drop bounds checks.
            let coefficient_row =
                &mut coefficient_row[(start_x - u0) as usize..][..(num_u - 1) as usize * 2 + 1];

            for u_b in 0..num_u {
                coefficient_row[u_b as usize * 2] = idwt_input.coefficients[(v_b
                    + idwt_input.padding.top)
                    * input_total_width as usize
                    + u_b as usize
                    + input_left_padding];
            }
        }
    }

    interleaved
}

/// The HOR_SR procedure from F.3.4.
fn filter_horizontal(samples: &mut InterleavedSamples, rect: IntRect, transform: WaveletTransform) {
    let total_width = rect.width() as usize + samples.padding.left + samples.padding.right;

    for scanline in samples
        .coefficients
        .chunks_exact_mut(total_width)
        .skip(samples.padding.top)
        .take(rect.height() as usize)
    {
        filter_single_row(
            scanline,
            samples.padding.left,
            samples.padding.left + rect.width() as usize,
            transform,
        );
    }
}

/// The VER_SR procedure from F.3.5.
fn filter_vertical(
    samples: &mut InterleavedSamples,
    temp_buf: &mut Vec<f32>,
    rect: IntRect,
    transform: WaveletTransform,
) {
    let total_width = rect.width() as usize + samples.padding.left + samples.padding.right;
    let total_height = rect.height() as usize + samples.padding.top + samples.padding.bottom;

    for u in samples.padding.left..(rect.width() as usize + samples.padding.left) {
        temp_buf.clear();

        // Extract column into buffer.
        for y in 0..total_height {
            temp_buf.push(samples.coefficients[u + total_width * y]);
        }

        filter_single_row(
            temp_buf,
            samples.padding.top,
            samples.padding.top + rect.height() as usize,
            transform,
        );

        // Put values back into original array.
        for (y, item) in temp_buf.iter().enumerate().take(total_height) {
            samples.coefficients[u + total_width * y] = *item;
        }
    }
}

/// The 1D_SR procedure from F.3.6.
fn filter_single_row(scanline: &mut [f32], start: usize, end: usize, transform: WaveletTransform) {
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
    // Hint the compiler that we won't go OOB to emit bound checks.
    let scanline = &mut scanline[..2 * (end / 2 + 1)];

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

    // Hint the compiler that we won't go OOB to emit bound checks.
    let scanline = &mut scanline[..2 * (end / 2 + 2)];

    // Step 1.
    for i in (start / 2 - 1)..(end / 2 + 2) {
        scanline[2 * i] *= KAPPA;
    }

    // Step 2.
    for i in (start / 2 - 2)..(end / 2 + 2) {
        scanline[2 * i + 1] *= 1.0 / KAPPA;
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
fn extend_signal(scanline: &mut [f32], start: usize, end: usize, transform: WaveletTransform) {
    let i_left = left_extension(transform, start);
    let i_right = right_extension(transform, end);

    for i in (start - i_left)..start {
        scanline[i] = scanline[periodic_symmetric_extension(i, start, end)];
    }

    for i in end..(end + i_right) {
        scanline[i] = scanline[periodic_symmetric_extension(i, start, end)];
    }
}

fn left_extension(transform: WaveletTransform, start: usize) -> usize {
    // Table F.2.
    match transform {
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
    }
}

fn right_extension(transform: WaveletTransform, end: usize) -> usize {
    // Table F.3.
    match transform {
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
        super::extend_signal(&mut data, 3, 9, WaveletTransform::Reversible53);

        assert_eq!(
            data,
            [0.0, 3.0, 2.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 0.0]
        );
    }
}
