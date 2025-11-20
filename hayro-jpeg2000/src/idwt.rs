//! Performing the inverse discrete wavelet transform, as specified in Annex F.

use crate::codestream::WaveletTransform;
use crate::decode::{Decomposition, SubBand, SubBandType};
use crate::rect::IntRect;

// Keep in sync with the type `F32` in the `simd` module!
const SIMD_WIDTH: usize = 8;

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
    // The buffer that will hold the final coefficients.
    pub(crate) coefficients: Vec<f32>,
    /// The size of the padding applied to each side.
    pub(crate) padding: Padding,
    /// The rect that the coefficients belong to. This will be equivalent
    /// to the rectangle that forms the smallest decomposition level. It does
    /// not have to be equivalent to the original size of the tile, as the
    /// sub-bands that form a tile aren't necessarily aligned to it. Therefore,
    /// the samples need to be trimmed to the tile rectangle afterward.
    pub(crate) rect: IntRect,
}

impl IDWTOutput {
    pub(crate) fn dummy() -> Self {
        Self {
            coefficients: vec![],
            padding: Default::default(),
            rect: IntRect::from_ltrb(0, 0, u32::MAX, u32::MAX),
        }
    }
}

impl IDWTOutput {
    pub(crate) fn total_width(&self) -> u32 {
        self.padding.left as u32 + self.rect.width() + self.padding.right as u32
    }
}

struct IDWTTempOutput {
    pub(crate) padding: Padding,
    pub(crate) rect: IntRect,
}

/// Apply the inverse discrete wavelet transform (see Annex F). The output
/// will be transformed samples covering the rectangle of the smallest
/// decomposition level.
pub(crate) fn apply(
    // The LL sub-band for resolution level 0.
    ll_sub_band: &SubBand,
    // All decomposition level that make up the tile.
    decompositions: &[Decomposition],
    // The buffer containing all sub-bands, used for resolving the sub-bands
    // of each decomposition level.
    sub_bands: &[SubBand],
    scratch_buffer: &mut Vec<f32>,
    output: &mut IDWTOutput,
    transform: WaveletTransform,
    sub_bands_coefficients: &[f32],
) {
    // To explain a bit why we have this scratch buffer and another coefficient
    // buffer: During IDWT, we need to continuously interleave the 4 sub-bands
    // into a new buffer, which is then either returned or used as the input
    // for the next decomposition, etc. It would be very inefficient if we
    // kept allocating new buffers each time. Therefore, we try to reuse them,
    // not only for all decompositions of a single tile, but all decompositions
    // of _all_ tiles.
    // Due to the fact that the output from the previous iteration might be
    // used as the input of the next, we need two separate buffers, which
    // are continuously swapped.
    let (scratch, coefficients) = (scratch_buffer, &mut output.coefficients);

    let estimate_buffer_size = |decomposition: &Decomposition| {
        // The maximum padding size (determined by
        // `left_extension`/`right_extension`) is 4 + 1 = 5.
        const MAX_PADDING: usize = 5;
        // For the width, we also need to account for additional padding on the
        // right side added for SIMD (see `interleave_samples`).
        let total_width =
            MAX_PADDING + decomposition.rect.width() as usize + MAX_PADDING + SIMD_WIDTH;
        let total_height = MAX_PADDING + decomposition.rect.height() as usize + MAX_PADDING;

        let min = total_width * total_height;
        // Different sub-bands can have shifts by one, so add even more padding
        // for the maximum case.
        let max = (total_width + 1) * (total_height + 1);

        (min, max)
    };

    if decompositions.is_empty() {
        // Single decomposition, just copy the coefficients from the sub-band.
        coefficients.clear();
        coefficients.extend_from_slice(&sub_bands_coefficients[ll_sub_band.coefficients.clone()]);

        output.padding = Padding::default();
        output.rect = ll_sub_band.rect;

        return;
    }

    // The coefficient array will always be the one that holds the coefficients
    // from the highest decomposition. Therefore, reserve as much.
    let (s_min, s_max) = estimate_buffer_size(decompositions.last().unwrap());
    if coefficients.capacity() < s_min {
        coefficients.reserve_exact(s_max - coefficients.capacity());
    }

    if decompositions.len() > 1 {
        // Due to the above, the intermediate buffer will never need more than
        // the second-highest decomposition.
        let (s_min, s_max) = estimate_buffer_size(&decompositions[decompositions.len() - 2]);

        if scratch.capacity() < s_min {
            scratch.reserve_exact(s_max - scratch.capacity());
        }
    }

    // Determine which buffer we should use first, such that the `coefficients`
    // array will always hold the final values.
    let mut use_scratch = decompositions.len().is_multiple_of(2);

    let mut temp_output = filter_2d(
        IDWTInput::from_sub_band(ll_sub_band, sub_bands_coefficients),
        if use_scratch { scratch } else { coefficients },
        &decompositions[0],
        transform,
        sub_bands,
        sub_bands_coefficients,
    );

    for decomposition in decompositions.iter().skip(1) {
        use_scratch = !use_scratch;

        temp_output = if use_scratch {
            filter_2d(
                IDWTInput::from_output(&temp_output, coefficients),
                scratch,
                decomposition,
                transform,
                sub_bands,
                sub_bands_coefficients,
            )
        } else {
            filter_2d(
                IDWTInput::from_output(&temp_output, scratch),
                coefficients,
                decomposition,
                transform,
                sub_bands,
                sub_bands_coefficients,
            )
        };
    }

    output.rect = temp_output.rect;
    output.padding = temp_output.padding;
}

struct IDWTInput<'a> {
    coefficients: &'a [f32],
    padding: Padding,
    sub_band_type: SubBandType,
}

impl<'a> IDWTInput<'a> {
    fn from_sub_band(sub_band: &'a SubBand, sub_band_coefficients: &'a [f32]) -> IDWTInput<'a> {
        IDWTInput {
            coefficients: &sub_band_coefficients[sub_band.coefficients.clone()],
            padding: Padding::default(),
            sub_band_type: sub_band.sub_band_type,
        }
    }

    fn from_output(idwt_output: &'a IDWTTempOutput, coefficients: &'a [f32]) -> IDWTInput<'a> {
        IDWTInput {
            coefficients,
            padding: idwt_output.padding,
            // The output from a previous iteration turns into the LL sub band
            // for the next iteration.
            sub_band_type: SubBandType::LowLow,
        }
    }
}

/// The 2D_SR procedure illustrated in Figure F.6.
fn filter_2d(
    // The LL sub band of the given decomposition level.
    input: IDWTInput,
    coefficients: &mut Vec<f32>,
    decomposition: &Decomposition,
    transform: WaveletTransform,
    sub_bands: &[SubBand],
    sub_band_coefficients: &[f32],
) -> IDWTTempOutput {
    // First interleave all of the sub-bands into a single buffer. We also
    // apply a padding so that we can transparently deal with border values.
    let padding = interleave_samples(
        input,
        decomposition,
        sub_bands,
        coefficients,
        transform,
        sub_band_coefficients,
    );

    if decomposition.rect.width() > 0 && decomposition.rect.height() > 0 {
        filter_horizontal(coefficients, padding, decomposition.rect, transform);
        simd::filter_vertical_simd(coefficients, padding, decomposition.rect, transform);
    }

    IDWTTempOutput {
        rect: decomposition.rect,
        padding,
    }
}

/// The 2D_INTERLEAVE procedure described in F.3.3.
fn interleave_samples(
    input: IDWTInput,
    decomposition: &Decomposition,
    sub_bands: &[SubBand],
    coefficients: &mut Vec<f32>,
    transform: WaveletTransform,
    sub_bands_coefficients: &[f32],
) -> Padding {
    let new_padding = {
        // The reason why we need + 1 for the left and top padding is very
        // subtle. In general, the methods return how many indices to the
        // left of the border can possibly be accessed. This is dependent
        // on the wavelet transform but also whether the start index (indicated
        // by the rect of the decomposition) is even or odd.
        //
        // For example, let's say we are using the 5-3 transform and our index
        // is even. According to the table, we need a padding of one to the left.
        // This makes sense, because our `base_idx` in 5-3 is (start / 2) * 2.
        // And the lowest access is `base_idx - 1`. So, for example, if start is
        // 2, then:
        // base_idx = (2 / 2) * 2 = 2,
        // and our lowest access is 1, so a padding of 1 is sufficient.
        // However, if we were to add only a padding of 1, our previously even
        // index now becomes uneven (3), and the previous math doesn't work
        // anymore since the evenness changed. Now, if we rerun the calculation:
        // base_idx = (3 / 2) * 2 = 2,
        // and our lowest access is therefore 1 again, which represents a delta
        // of 2 instead of the previously calculated 1.
        // Therefore, we always need to add a padding of 1 to the top and
        // left to prevent OOB accesses.
        let left_padding = left_extension(transform, decomposition.rect.x0 as usize) + 1;
        let top_padding = left_extension(transform, decomposition.rect.y0 as usize) + 1;
        let mut right_padding = right_extension(transform, decomposition.rect.x1 as usize);
        let bottom_padding = right_extension(transform, decomposition.rect.y1 as usize);

        // For vertical filtering, we use SIMD to process multiple columns at
        // the same time. Therefore, we add additional padding to the
        // right such that we can always iterate in chunks of our SIMD width
        // without having to deal with any remainder.
        let current_width = left_padding + decomposition.rect.width() as usize + right_padding;
        let target_width = current_width.next_multiple_of(SIMD_WIDTH);
        right_padding += target_width - current_width;

        Padding::new(left_padding, top_padding, right_padding, bottom_padding)
    };

    let total_width = decomposition.rect.width() as usize + new_padding.left + new_padding.right;
    let total_height = decomposition.rect.height() as usize + new_padding.top + new_padding.bottom;

    // Just a sanity check. We should have allocated enough upfront before
    // starting the IDWT.
    assert!(coefficients.capacity() >= total_width * total_height);

    // The cleaner way would be to first clear and then resize, so that we
    // have a clean buffer with just zeroes. However, this is actually not
    // necessary, because when interleaving and generating the border values
    // we will replace all the data anyway, so we can save the cost of
    // the clear operation.
    coefficients.resize(total_width * total_height, 0.0);

    let IntRect {
        x0: u0,
        x1: u1,
        y0: v0,
        y1: v1,
    } = decomposition.rect;

    // Perform the actual interleaving of sub-bands, taking the padding into
    // account.
    for idwt_input in [
        input,
        IDWTInput::from_sub_band(
            &sub_bands[decomposition.sub_bands[0]],
            sub_bands_coefficients,
        ),
        IDWTInput::from_sub_band(
            &sub_bands[decomposition.sub_bands[1]],
            sub_bands_coefficients,
        ),
        IDWTInput::from_sub_band(
            &sub_bands[decomposition.sub_bands[2]],
            sub_bands_coefficients,
        ),
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

        let coefficient_rows = coefficients
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

    new_padding
}

/// The HOR_SR procedure from F.3.4.
fn filter_horizontal(
    coefficients: &mut [f32],
    padding: Padding,
    rect: IntRect,
    transform: WaveletTransform,
) {
    let total_width = rect.width() as usize + padding.left + padding.right;

    for scanline in coefficients
        .chunks_exact_mut(total_width)
        .skip(padding.top)
        .take(rect.height() as usize)
    {
        filter_single_row(
            scanline,
            padding.left,
            padding.left + rect.width() as usize,
            transform,
        );
    }
}

/// The VER_SR procedure from F.3.5.
#[allow(dead_code)]
fn filter_vertical(
    coefficients: &mut [f32],
    padding: Padding,
    temp_buf: &mut Vec<f32>,
    rect: IntRect,
    transform: WaveletTransform,
) {
    let total_width = rect.width() as usize + padding.left + padding.right;
    let total_height = rect.height() as usize + padding.top + padding.bottom;

    for u in padding.left..(rect.width() as usize + padding.left) {
        temp_buf.clear();

        // Extract column into buffer.
        for y in 0..total_height {
            temp_buf.push(coefficients[u + total_width * y]);
        }

        filter_single_row(
            temp_buf,
            padding.top,
            padding.top + rect.height() as usize,
            transform,
        );

        // Put values back into original array.
        for (y, item) in temp_buf.iter().enumerate().take(total_height) {
            coefficients[u + total_width * y] = *item;
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

mod simd {
    use crate::codestream::WaveletTransform;
    use crate::idwt::{Padding, left_extension, periodic_symmetric_extension, right_extension};
    use crate::rect::IntRect;
    use fearless_simd::*;

    const SIMD_WIDTH: usize = super::SIMD_WIDTH;
    type F32<S> = f32x8<S>;

    pub(super) fn filter_vertical_simd(
        coefficients: &mut [f32],
        padding: Padding,
        rect: IntRect,
        transform: WaveletTransform,
    ) {
        let level = Level::new();
        dispatch!(level, simd => filter_vertical_simd_impl(simd, coefficients, padding, rect, transform));
    }

    /// The VER_SR procedure from F.3.5.
    #[inline(always)]
    fn filter_vertical_simd_impl<S: Simd>(
        simd: S,
        coefficients: &mut [f32],
        padding: Padding,
        rect: IntRect,
        transform: WaveletTransform,
    ) {
        let total_width = rect.width() as usize + padding.left + padding.right;
        filter_rows_simd(
            simd,
            coefficients,
            padding.top,
            padding.top + rect.height() as usize,
            total_width,
            transform,
        );
    }

    /// The 1D_SR procedure from F.3.6.
    #[inline(always)]
    fn filter_rows_simd<S: Simd>(
        simd: S,
        scanline: &mut [f32],
        start: usize,
        end: usize,
        stride: usize,
        transform: WaveletTransform,
    ) {
        if start == end - 1 {
            if !start.is_multiple_of(2) {
                for base_column in (0..stride).step_by(SIMD_WIDTH) {
                    let mut loaded = F32::from_slice(
                        simd,
                        &scanline[(start * stride) + base_column..][..SIMD_WIDTH],
                    );
                    loaded /= 2.0;
                    scanline[(start * stride) + base_column..][..SIMD_WIDTH]
                        .copy_from_slice(&loaded.val);
                }
            }

            return;
        }

        extend_signal_simd(simd, scanline, start, end, stride, transform);

        match transform {
            WaveletTransform::Reversible53 => {
                reversible_filter_53r_simd(simd, scanline, start, end, stride);
            }
            WaveletTransform::Irreversible97 => {
                irreversible_filter_97i_simd(simd, scanline, start, end, stride);
            }
        }
    }

    /// The 1D_EXTR procedure, defined in F.3.7.
    #[inline(always)]
    fn extend_signal_simd<S: Simd>(
        simd: S,
        scanline: &mut [f32],
        start: usize,
        end: usize,
        stride: usize,
        transform: WaveletTransform,
    ) {
        let i_left = left_extension(transform, start);
        let i_right = right_extension(transform, end);

        for i in (start - i_left)..start {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let idx = periodic_symmetric_extension(i, start, end);
                let loaded =
                    F32::from_slice(simd, &scanline[idx * stride + base_column..][..SIMD_WIDTH]);
                scanline[i * stride + base_column..][..SIMD_WIDTH].copy_from_slice(&loaded.val);
            }
        }

        for i in end..(end + i_right) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let idx = periodic_symmetric_extension(i, start, end);
                let loaded =
                    F32::from_slice(simd, &scanline[idx * stride + base_column..][..SIMD_WIDTH]);
                scanline[i * stride + base_column..][..SIMD_WIDTH].copy_from_slice(&loaded.val);
            }
        }
    }

    /// The 1D FILTER 5-3R procedure from F.3.8.1.
    #[inline(always)]
    fn reversible_filter_53r_simd<S: Simd>(
        simd: S,
        scanline: &mut [f32],
        start: usize,
        end: usize,
        stride: usize,
    ) {
        // Equation (F-5).
        for n in start / 2..(end / 2) + 1 {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let base_idx = 2 * n * stride + base_column;

                let mut s1 = F32::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
                let s2 = F32::from_slice(simd, &scanline[base_idx - stride..][..SIMD_WIDTH]);
                let s3 = F32::from_slice(simd, &scanline[base_idx + stride..][..SIMD_WIDTH]);

                s1 -= ((s2 + s3 + 2.0) / 4.0).floor();

                scanline[base_idx..][..SIMD_WIDTH].copy_from_slice(&s1.val);
            }
        }

        // Equation (F-6).
        for n in start / 2..(end / 2) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let base_idx = (2 * n + 1) * stride + base_column;

                let mut s1 = F32::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
                let s2 = F32::from_slice(simd, &scanline[base_idx - stride..][..SIMD_WIDTH]);
                let s3 = F32::from_slice(simd, &scanline[base_idx + stride..][..SIMD_WIDTH]);

                s1 += ((s2 + s3) / 2.0).floor();

                scanline[base_idx..][..SIMD_WIDTH].copy_from_slice(&s1.val);
            }
        }
    }

    /// The 1D Filter 9-7I procedure from F.3.8.2 executed with SIMD.
    #[inline(always)]
    fn irreversible_filter_97i_simd<S: Simd>(
        simd: S,
        scanline: &mut [f32],
        start: usize,
        end: usize,
        stride: usize,
    ) {
        const ALPHA: f32 = -1.586_134_3;
        const BETA: f32 = -0.052_980_117;
        const GAMMA: f32 = 0.882_911_1;
        const DELTA: f32 = 0.443_506_87;
        const KAPPA: f32 = 1.230_174_1;

        let alpha = F32::splat(simd, ALPHA);
        let beta = F32::splat(simd, BETA);
        let gamma = F32::splat(simd, GAMMA);
        let delta = F32::splat(simd, DELTA);
        let kappa = F32::splat(simd, KAPPA);
        let inv_kappa = F32::splat(simd, 1.0 / KAPPA);

        // Step 1.
        for i in (start / 2 - 1)..(end / 2 + 2) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let base_idx = (2 * i) * stride + base_column;
                let mut vals = F32::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
                vals *= kappa;
                scanline[base_idx..][..SIMD_WIDTH].copy_from_slice(&vals.val);
            }
        }

        // Step 2.
        for i in (start / 2 - 2)..(end / 2 + 2) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let base_idx = (2 * i + 1) * stride + base_column;
                let mut vals = F32::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
                vals *= inv_kappa;
                scanline[base_idx..][..SIMD_WIDTH].copy_from_slice(&vals.val);
            }
        }

        // Step 3.
        for i in (start / 2 - 1)..(end / 2 + 2) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let base_idx = (2 * i) * stride + base_column;

                let mut s1 = F32::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
                let s2 = F32::from_slice(simd, &scanline[base_idx - stride..][..SIMD_WIDTH]);
                let s3 = F32::from_slice(simd, &scanline[base_idx + stride..][..SIMD_WIDTH]);

                s1 -= delta * (s2 + s3);
                scanline[base_idx..][..SIMD_WIDTH].copy_from_slice(&s1.val);
            }
        }

        // Step 4.
        for i in (start / 2 - 1)..(end / 2 + 1) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let base_idx = (2 * i + 1) * stride + base_column;

                let mut s1 = F32::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
                let s2 = F32::from_slice(simd, &scanline[base_idx - stride..][..SIMD_WIDTH]);
                let s3 = F32::from_slice(simd, &scanline[base_idx + stride..][..SIMD_WIDTH]);

                s1 -= gamma * (s2 + s3);
                scanline[base_idx..][..SIMD_WIDTH].copy_from_slice(&s1.val);
            }
        }

        // Step 5.
        for i in (start / 2)..(end / 2 + 1) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let base_idx = (2 * i) * stride + base_column;

                let mut s1 = F32::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
                let s2 = F32::from_slice(simd, &scanline[base_idx - stride..][..SIMD_WIDTH]);
                let s3 = F32::from_slice(simd, &scanline[base_idx + stride..][..SIMD_WIDTH]);

                s1 -= beta * (s2 + s3);
                scanline[base_idx..][..SIMD_WIDTH].copy_from_slice(&s1.val);
            }
        }

        // Step 6.
        for i in (start / 2)..(end / 2) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let base_idx = (2 * i + 1) * stride + base_column;

                let mut s1 = F32::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
                let s2 = F32::from_slice(simd, &scanline[base_idx - stride..][..SIMD_WIDTH]);
                let s3 = F32::from_slice(simd, &scanline[base_idx + stride..][..SIMD_WIDTH]);

                s1 -= alpha * (s2 + s3);
                scanline[base_idx..][..SIMD_WIDTH].copy_from_slice(&s1.val);
            }
        }
    }
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
