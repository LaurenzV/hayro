//! Performing the inverse discrete wavelet transform, as specified in Annex F.

use super::build::{Decomposition, SubBand, SubBandType};
use super::codestream::WaveletTransform;
use super::decode::{DecompositionStorage, TileDecodeContext};
use super::rect::IntRect;
use super::simd::{Level, SIMD_WIDTH, Simd, dispatch, f32x8};
use crate::j2c::Header;

#[derive(Default, Copy, Clone)]
pub(crate) struct Padding {
    pub(crate) right: usize,
}

/// The output from performing the IDWT operation.
pub(crate) struct IDWTOutput {
    /// The buffer that will hold the final coefficients.
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
            padding: Padding::default(),
            rect: IntRect::from_ltrb(0, 0, u32::MAX, u32::MAX),
        }
    }
}

impl IDWTOutput {
    /// The total width of the output, including padding.
    pub(crate) fn total_width(&self) -> u32 {
        self.rect.width() + self.padding.right as u32
    }
}

struct IDWTTempOutput {
    padding: Padding,
    rect: IntRect,
}

/// Apply the inverse discrete wavelet transform (see Annex F). The output
/// will be transformed samples covering the rectangle of the smallest
/// decomposition level.
pub(crate) fn apply(
    storage: &DecompositionStorage<'_>,
    tile_ctx: &mut TileDecodeContext<'_>,
    component_idx: usize,
    header: &Header<'_>,
    transform: WaveletTransform,
) {
    let tile_decompositions = &storage.tile_decompositions[component_idx];

    let mut decompositions = &storage.decompositions[tile_decompositions.decompositions.clone()];
    // If we requested a lower resolution level, we can skip some decompositions.
    decompositions = &decompositions[..decompositions
        .len()
        .saturating_sub(header.skipped_resolution_levels as usize)];
    let ll_sub_band = &storage.sub_bands[tile_decompositions.first_ll_sub_band];

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
    let (scratch_buf, output) = (&mut tile_ctx.idwt_scratch_buffer, &mut tile_ctx.idwt_output);

    let estimate_buffer_size = |decomposition: &Decomposition| {
        // For the width, we need to account for SIMD padding on the right.
        let total_width = decomposition.rect.width() as usize + SIMD_WIDTH;
        let total_height = decomposition.rect.height() as usize;

        let min = total_width * total_height;
        // Different sub-bands can have shifts by one, so add even more padding
        // for the maximum case.
        let max = (total_width + 1) * (total_height + 1);

        (min, max)
    };

    if decompositions.is_empty() {
        // Single decomposition, just copy the coefficients from the sub-band.
        output.coefficients.clear();
        output
            .coefficients
            .extend_from_slice(&storage.coefficients[ll_sub_band.coefficients.clone()]);

        output.padding = Padding::default();
        output.rect = ll_sub_band.rect;

        return;
    }

    // The coefficient array will always be the one that holds the coefficients
    // from the highest decomposition. Therefore, reserve as much.
    let (s_min, s_max) = estimate_buffer_size(decompositions.last().unwrap());
    if output.coefficients.len() < s_min {
        output
            .coefficients
            .reserve_exact(s_max - output.coefficients.len());
    }

    if decompositions.len() > 1 {
        // Due to the above, the intermediate buffer will never need more than
        // the second-highest decomposition.
        let (s_min, s_max) = estimate_buffer_size(&decompositions[decompositions.len() - 2]);

        if scratch_buf.len() < s_min {
            scratch_buf.reserve_exact(s_max - scratch_buf.len());
        }
    }

    // Determine which buffer we should use first, such that the `coefficients`
    // array will always hold the final values.
    let mut use_scratch = decompositions.len().is_multiple_of(2);

    let mut temp_output = filter_2d(
        IDWTInput::from_sub_band(ll_sub_band, storage),
        if use_scratch {
            scratch_buf
        } else {
            &mut output.coefficients
        },
        &decompositions[0],
        transform,
        storage,
    );

    for decomposition in decompositions.iter().skip(1) {
        use_scratch = !use_scratch;

        temp_output = if use_scratch {
            filter_2d(
                IDWTInput::from_output(&temp_output, &output.coefficients),
                scratch_buf,
                decomposition,
                transform,
                storage,
            )
        } else {
            filter_2d(
                IDWTInput::from_output(&temp_output, scratch_buf),
                &mut output.coefficients,
                decomposition,
                transform,
                storage,
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
    fn from_sub_band(sub_band: &'a SubBand, storage: &'a DecompositionStorage<'_>) -> Self {
        IDWTInput {
            coefficients: &storage.coefficients[sub_band.coefficients.clone()],
            padding: Padding::default(),
            sub_band_type: sub_band.sub_band_type,
        }
    }

    fn from_output(idwt_output: &'a IDWTTempOutput, coefficients: &'a [f32]) -> Self {
        IDWTInput {
            coefficients,
            padding: idwt_output.padding,
            // The output from a previous iteration turns into the LL sub band
            // for the next iteration.
            sub_band_type: SubBandType::LowLow,
        }
    }
}

/// The `2D_SR` procedure illustrated in Figure F.6.
fn filter_2d(
    // The LL sub band of the given decomposition level.
    input: IDWTInput<'_>,
    coefficients: &mut Vec<f32>,
    decomposition: &Decomposition,
    transform: WaveletTransform,
    storage: &DecompositionStorage<'_>,
) -> IDWTTempOutput {
    // First interleave all sub-bands into a single buffer. We also
    // apply a padding so that we can transparently deal with border values.
    let padding = interleave_samples(input, decomposition, coefficients, storage);

    if decomposition.rect.width() > 0 && decomposition.rect.height() > 0 {
        filter_horizontal(coefficients, padding, decomposition.rect, transform);
        filter_vertical(coefficients, padding, decomposition.rect, transform);
    }

    IDWTTempOutput {
        rect: decomposition.rect,
        padding,
    }
}

/// The `2D_INTERLEAVE` procedure described in F.3.3.
fn interleave_samples(
    input: IDWTInput<'_>,
    decomposition: &Decomposition,
    coefficients: &mut Vec<f32>,
    storage: &DecompositionStorage<'_>,
) -> Padding {
    let new_padding = {
        // For vertical filtering, we use SIMD to process multiple columns at
        // the same time. Therefore, we add padding to the right such that we
        // can always iterate in chunks of our SIMD width without having to
        // deal with any remainder.
        let current_width = decomposition.rect.width() as usize;
        let target_width = current_width.next_multiple_of(SIMD_WIDTH);
        let right_padding = target_width - current_width;

        Padding {
            right: right_padding,
        }
    };

    let total_width = decomposition.rect.width() as usize + new_padding.right;
    let total_height = decomposition.rect.height() as usize;

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
        IDWTInput::from_sub_band(&storage.sub_bands[decomposition.sub_bands[0]], storage),
        IDWTInput::from_sub_band(&storage.sub_bands[decomposition.sub_bands[1]], storage),
        IDWTInput::from_sub_band(&storage.sub_bands[decomposition.sub_bands[2]], storage),
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

        let input_total_width = num_u as usize + idwt_input.padding.right;

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
            .map(|s| &mut s[..decomposition.rect.width() as usize])
            .skip((start_y - v0) as usize)
            .step_by(2);

        for (v_b, coefficient_row) in coefficient_rows.enumerate().take(num_v as usize) {
            // Hint compiler to drop bounds checks.
            let coefficient_row =
                &mut coefficient_row[(start_x - u0) as usize..][..(num_u - 1) as usize * 2 + 1];

            for u_b in 0..num_u {
                coefficient_row[u_b as usize * 2] =
                    idwt_input.coefficients[v_b * input_total_width + u_b as usize];
            }
        }
    }

    new_padding
}

/// The `HOR_SR` procedure from F.3.4.
fn filter_horizontal(
    coefficients: &mut [f32],
    padding: Padding,
    rect: IntRect,
    transform: WaveletTransform,
) {
    let width = rect.width() as usize;
    let total_width = width + padding.right;

    for scanline in coefficients
        .chunks_exact_mut(total_width)
        .take(rect.height() as usize)
    {
        filter_row(&mut scanline[..width], width, rect.x0 as usize, transform);
    }
}

/// The `1D_SR` procedure from F.3.6.
fn filter_row(scanline: &mut [f32], width: usize, x0: usize, transform: WaveletTransform) {
    if width == 1 {
        if !x0.is_multiple_of(2) {
            scanline[0] /= 2.0;
        }

        return;
    }

    match transform {
        WaveletTransform::Reversible53 => reversible_filter_53r(scanline, width, x0),
        WaveletTransform::Irreversible97 => irreversible_filter_97i(scanline, width, x0),
    }
}

/// The 1D FILTER 5-3R procedure from F.3.8.1.
fn reversible_filter_53r(scanline: &mut [f32], width: usize, x0: usize) {
    // Note that this for loop does not match exactly what's in the reference.
    // There is a clever subtlety that we can make use of to make the loop shorter.
    //
    // In the reference, the presented semantics of IDWT is that we explicitly
    // store the left/right padding in an array. As part of the for loop, we will
    // first modify an out-of-bound column (conceptually at the relative location
    // -1) before proceeding to the columns that are actually inside of the image.
    // However, the key insight is that for example the column at location -1 will
    // actually have the same filtered values as the column at location of 1 due
    // to reflection. The same applies to -3 and 3, etc.
    // Therefore, as long as we properly reflect the lower and upper indices,
    // we don't need to compute and store those boundary values explicitly.
    //
    // The above comment also applies to the 9-7 filter.

    // Indices depend on whether the _global_ start coordinate is even or odd.
    let first_even = x0 % 2;
    let first_odd = 1 - first_even;

    // Equation (F-5).
    // Originally: for i in (start / 2)..(end / 2 + 1).
    for i in (first_even..width).step_by(2) {
        let left = periodic_symmetric_extension(i, -1, width);
        let right = periodic_symmetric_extension(i, 1, width);
        scanline[i] -= ((scanline[left] + scanline[right] + 2.0) * 0.25).floor();
    }

    // Equation (F-6).
    // Originally: for i in (start / 2)..(end / 2).
    for i in (first_odd..width).step_by(2) {
        let left = periodic_symmetric_extension(i, -1, width);
        let right = periodic_symmetric_extension(i, 1, width);
        scanline[i] += ((scanline[left] + scanline[right]) * 0.5).floor();
    }
}

/// The 1D Filter 9-7I procedure from F.3.8.2.
fn irreversible_filter_97i(scanline: &mut [f32], width: usize, x0: usize) {
    // Table F.4.
    const ALPHA: f32 = -1.586_134_3;
    const BETA: f32 = -0.052_980_117;
    const GAMMA: f32 = 0.882_911_1;
    const DELTA: f32 = 0.443_506_87;
    const KAPPA: f32 = 1.230_174_1;
    const INV_KAPPA: f32 = 1.0 / KAPPA;

    let first_even = x0 % 2;
    let first_odd = 1 - first_even;

    // Step 1.
    // Originally: for i in (start / 2 - 1)..(end / 2 + 2).
    for i in (first_even..width).step_by(2) {
        scanline[i] *= KAPPA;
    }

    // Step 2.
    // Originally: for i in (start / 2 - 2)..(end / 2 + 2).
    for i in (first_odd..width).step_by(2) {
        scanline[i] *= INV_KAPPA;
    }

    // Step 3.
    // Originally: for i in (start / 2 - 1)..(end / 2 + 2).
    for i in (first_even..width).step_by(2) {
        let left = periodic_symmetric_extension(i, -1, width);
        let right = periodic_symmetric_extension(i, 1, width);
        scanline[i] -= DELTA * (scanline[left] + scanline[right]);
    }

    // Step 4.
    // Originally: for i in (start / 2 - 1)..((x0 + width) / 2 + 1).
    for i in (first_odd..width).step_by(2) {
        let left = periodic_symmetric_extension(i, -1, width);
        let right = periodic_symmetric_extension(i, 1, width);
        scanline[i] -= GAMMA * (scanline[left] + scanline[right]);
    }

    // Step 5.
    // Originally: for i in (start / 2)..(end / 2 + 1).
    for i in (first_even..width).step_by(2) {
        let left = periodic_symmetric_extension(i, -1, width);
        let right = periodic_symmetric_extension(i, 1, width);
        scanline[i] -= BETA * (scanline[left] + scanline[right]);
    }

    // Step 6.
    // Originally: for i in (start / 2)..(end / 2).
    for i in (first_odd..width).step_by(2) {
        let left = periodic_symmetric_extension(i, -1, width);
        let right = periodic_symmetric_extension(i, 1, width);
        scanline[i] -= ALPHA * (scanline[left] + scanline[right]);
    }
}

/// Part of the `1D_EXTR` procedure, defined in F.3.7.
///
/// It performs a basic periodic symmetric extension. Our formula looks different
/// because we have no start offset and also want to avoid converting `usize`
/// to `isize` in case it's negative.
#[inline(always)]
fn periodic_symmetric_extension(idx: usize, offset: isize, length: usize) -> usize {
    if offset < 0 {
        let abs_offset = (-offset) as usize;
        abs_offset.abs_diff(idx)
    } else {
        let new_idx = idx + offset as usize;
        if new_idx >= length {
            let overshoot = new_idx - length;
            length - 2 - overshoot
        } else {
            new_idx
        }
    }
}

/// The `VER_SR` procedure from F.3.5.
fn filter_vertical(
    coefficients: &mut [f32],
    padding: Padding,
    rect: IntRect,
    transform: WaveletTransform,
) {
    dispatch!(Level::new(), simd => filter_vertical_impl(simd, coefficients, padding, rect, transform));
}

#[inline(always)]
fn filter_vertical_impl<S: Simd>(
    simd: S,
    scanline: &mut [f32],
    padding: Padding,
    rect: IntRect,
    transform: WaveletTransform,
) {
    let stride = rect.width() as usize + padding.right;
    let height = rect.height() as usize;
    let y0 = rect.y0 as usize;

    if height == 1 {
        if !y0.is_multiple_of(2) {
            for base_column in (0..stride).step_by(SIMD_WIDTH) {
                let mut loaded = f32x8::from_slice(simd, &scanline[base_column..][..SIMD_WIDTH]);
                loaded /= 2.0;
                loaded.store(&mut scanline[base_column..][..SIMD_WIDTH]);
            }
        }
        return;
    }

    match transform {
        WaveletTransform::Reversible53 => {
            reversible_filter_53r_simd(simd, scanline, height, stride, y0);
        }
        WaveletTransform::Irreversible97 => {
            irreversible_filter_97i_simd(simd, scanline, height, stride, y0);
        }
    }
}

/// The 1D FILTER 5-3R procedure from F.3.8.1.
#[inline(always)]
fn reversible_filter_53r_simd<S: Simd>(
    simd: S,
    scanline: &mut [f32],
    height: usize,
    stride: usize,
    y0: usize,
) {
    let first_even = y0 % 2;
    let first_odd = 1 - first_even;

    // Equation (F-5).
    // Originally: for i in (start / 2)..(end / 2 + 1).
    for row in (first_even..height).step_by(2) {
        let row_above = periodic_symmetric_extension(row, -1, height);
        let row_below = periodic_symmetric_extension(row, 1, height);

        for base_column in (0..stride).step_by(SIMD_WIDTH) {
            let mut s1 =
                f32x8::from_slice(simd, &scanline[row * stride + base_column..][..SIMD_WIDTH]);
            let s2 = f32x8::from_slice(
                simd,
                &scanline[row_above * stride + base_column..][..SIMD_WIDTH],
            );
            let s3 = f32x8::from_slice(
                simd,
                &scanline[row_below * stride + base_column..][..SIMD_WIDTH],
            );

            s1 -= ((s2 + s3 + 2.0) * 0.25).floor();
            s1.store(&mut scanline[row * stride + base_column..][..SIMD_WIDTH]);
        }
    }

    // Equation (F-6).
    // Originally: for i in (start / 2)..(end / 2).
    for row in (first_odd..height).step_by(2) {
        let row_above = periodic_symmetric_extension(row, -1, height);
        let row_below = periodic_symmetric_extension(row, 1, height);

        for base_column in (0..stride).step_by(SIMD_WIDTH) {
            let mut s1 =
                f32x8::from_slice(simd, &scanline[row * stride + base_column..][..SIMD_WIDTH]);
            let s2 = f32x8::from_slice(
                simd,
                &scanline[row_above * stride + base_column..][..SIMD_WIDTH],
            );
            let s3 = f32x8::from_slice(
                simd,
                &scanline[row_below * stride + base_column..][..SIMD_WIDTH],
            );

            s1 += ((s2 + s3) * 0.5).floor();
            s1.store(&mut scanline[row * stride + base_column..][..SIMD_WIDTH]);
        }
    }
}

/// The 1D Filter 9-7I procedure from F.3.8.2.
#[inline(always)]
fn irreversible_filter_97i_simd<S: Simd>(
    simd: S,
    scanline: &mut [f32],
    height: usize,
    stride: usize,
    y0: usize,
) {
    const ALPHA: f32 = -1.586_134_3;
    const BETA: f32 = -0.052_980_117;
    const GAMMA: f32 = 0.882_911_1;
    const DELTA: f32 = 0.443_506_87;
    const KAPPA: f32 = 1.230_174_1;

    const INV_KAPPA: f32 = 1.0 / KAPPA;

    let alpha = f32x8::splat(simd, ALPHA);
    let beta = f32x8::splat(simd, BETA);
    let gamma = f32x8::splat(simd, GAMMA);
    let delta = f32x8::splat(simd, DELTA);
    let kappa = f32x8::splat(simd, KAPPA);
    let inv_kappa = f32x8::splat(simd, INV_KAPPA);

    // Determine which local row indices correspond to even/odd global positions.
    let first_even = y0 % 2;
    let first_odd = 1 - first_even;

    // Step 1.
    // Originally: for i in (start / 2 - 1)..(end / 2 + 2).
    for row in (first_even..height).step_by(2) {
        for base_column in (0..stride).step_by(SIMD_WIDTH) {
            let base_idx = row * stride + base_column;
            let mut vals = f32x8::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
            vals = vals * kappa;
            vals.store(&mut scanline[base_idx..][..SIMD_WIDTH]);
        }
    }

    // Step 2.
    // Originally: for i in (start / 2 - 2)..(end / 2 + 2).
    for row in (first_odd..height).step_by(2) {
        for base_column in (0..stride).step_by(SIMD_WIDTH) {
            let base_idx = row * stride + base_column;
            let mut vals = f32x8::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
            vals = vals * inv_kappa;
            vals.store(&mut scanline[base_idx..][..SIMD_WIDTH]);
        }
    }

    // Step 3.
    // Originally: for i in (start / 2 - 1)..(end / 2 + 2).
    for row in (first_even..height).step_by(2) {
        let row_above = periodic_symmetric_extension(row, -1, height);
        let row_below = periodic_symmetric_extension(row, 1, height);

        for base_column in (0..stride).step_by(SIMD_WIDTH) {
            let base_idx = row * stride + base_column;

            let mut s1 = f32x8::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
            let s2 = f32x8::from_slice(
                simd,
                &scanline[row_above * stride + base_column..][..SIMD_WIDTH],
            );
            let s3 = f32x8::from_slice(
                simd,
                &scanline[row_below * stride + base_column..][..SIMD_WIDTH],
            );

            s1 -= delta * (s2 + s3);
            s1.store(&mut scanline[base_idx..][..SIMD_WIDTH]);
        }
    }

    // Step 4.
    // Originally: for i in (start / 2 - 1)..(end / 2 + 1).
    for row in (first_odd..height).step_by(2) {
        let row_above = periodic_symmetric_extension(row, -1, height);
        let row_below = periodic_symmetric_extension(row, 1, height);

        for base_column in (0..stride).step_by(SIMD_WIDTH) {
            let base_idx = row * stride + base_column;

            let mut s1 = f32x8::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
            let s2 = f32x8::from_slice(
                simd,
                &scanline[row_above * stride + base_column..][..SIMD_WIDTH],
            );
            let s3 = f32x8::from_slice(
                simd,
                &scanline[row_below * stride + base_column..][..SIMD_WIDTH],
            );

            s1 -= gamma * (s2 + s3);
            s1.store(&mut scanline[base_idx..][..SIMD_WIDTH]);
        }
    }

    // Step 5.
    // Originally: for i in (start / 2)..(end / 2 + 1).
    for row in (first_even..height).step_by(2) {
        let row_above = periodic_symmetric_extension(row, -1, height);
        let row_below = periodic_symmetric_extension(row, 1, height);

        for base_column in (0..stride).step_by(SIMD_WIDTH) {
            let base_idx = row * stride + base_column;

            let mut s1 = f32x8::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
            let s2 = f32x8::from_slice(
                simd,
                &scanline[row_above * stride + base_column..][..SIMD_WIDTH],
            );
            let s3 = f32x8::from_slice(
                simd,
                &scanline[row_below * stride + base_column..][..SIMD_WIDTH],
            );

            s1 -= beta * (s2 + s3);
            s1.store(&mut scanline[base_idx..][..SIMD_WIDTH]);
        }
    }

    // Step 6.
    // Originally: for i in (start / 2)..(end / 2).
    for row in (first_odd..height).step_by(2) {
        let row_above = periodic_symmetric_extension(row, -1, height);
        let row_below = periodic_symmetric_extension(row, 1, height);

        for base_column in (0..stride).step_by(SIMD_WIDTH) {
            let base_idx = row * stride + base_column;

            let mut s1 = f32x8::from_slice(simd, &scanline[base_idx..][..SIMD_WIDTH]);
            let s2 = f32x8::from_slice(
                simd,
                &scanline[row_above * stride + base_column..][..SIMD_WIDTH],
            );
            let s3 = f32x8::from_slice(
                simd,
                &scanline[row_below * stride + base_column..][..SIMD_WIDTH],
            );

            s1 -= alpha * (s2 + s3);
            s1.store(&mut scanline[base_idx..][..SIMD_WIDTH]);
        }
    }
}
