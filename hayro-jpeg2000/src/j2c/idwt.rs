use alloc::vec;
use alloc::vec::Vec;

use super::build::{Decomposition, SubBand};
use super::codestream::WaveletTransform;
use super::decode::{DecompositionStorage, TileDecodeContext};
use super::rect::IntRect;
use crate::cancel::Cancel;
use crate::error::Result;
use crate::j2c::Header;
use crate::math::{self, Level, SIMD_WIDTH, Simd, dispatch, f32x8};

pub(crate) struct IDWTOutput {
    pub(crate) coefficients: Vec<f32>,
    pub(crate) rect: IntRect,
}

impl Default for IDWTOutput {
    fn default() -> Self {
        Self {
            coefficients: vec![],
            rect: IntRect::from_ltrb(0, 0, u32::MAX, u32::MAX),
        }
    }
}

struct IDWTTempOutput {
    rect: IntRect,
}

pub(crate) fn apply(
    storage: &DecompositionStorage<'_>,
    tile_ctx: &mut TileDecodeContext,
    component_idx: usize,
    header: &Header,
    transform: WaveletTransform,
    cancel: Cancel<'_>,
) -> Result<()> {
    let tile_decompositions = &storage.tile_decompositions[component_idx];

    let mut decompositions = &storage.decompositions[tile_decompositions.decompositions.clone()];
    decompositions = &decompositions[..decompositions
        .len()
        .saturating_sub(header.skipped_resolution_levels as usize)];
    let ll_sub_band = &storage.sub_bands[tile_decompositions.first_ll_sub_band];

    let (scratch_buf, output) = (&mut tile_ctx.idwt_scratch_buffer, &mut tile_ctx.idwt_output);

    let estimate_buffer_size = |decomposition: &Decomposition| {
        let total_width = decomposition.rect.width() as usize;
        let total_height = decomposition.rect.height() as usize;

        let min = total_width * total_height;
        // Sub-bands can be offset by one sample.
        let max = (total_width + 1) * (total_height + 1);

        (min, max)
    };

    if decompositions.is_empty() {
        output.coefficients.clear();
        output
            .coefficients
            .extend_from_slice(&storage.coefficients[ll_sub_band.coefficients.clone()]);

        output.rect = ll_sub_band.rect;

        return Ok(());
    }

    let (s_min, s_max) = estimate_buffer_size(decompositions.last().unwrap());
    if output.coefficients.len() < s_min {
        output
            .coefficients
            .reserve_exact(s_max - output.coefficients.len());
    }

    if decompositions.len() > 1 {
        // The scratch buffer never holds more than the second-to-last decomposition.
        let (s_min, s_max) = estimate_buffer_size(&decompositions[decompositions.len() - 2]);

        if scratch_buf.len() < s_min {
            scratch_buf.reserve_exact(s_max - scratch_buf.len());
        }
    }

    // Start with the buffer that makes the last decomposition land in `coefficients`.
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
        cancel,
    )?;

    for decomposition in decompositions.iter().skip(1) {
        use_scratch = !use_scratch;

        temp_output = if use_scratch {
            filter_2d(
                IDWTInput::from_output(&output.coefficients),
                scratch_buf,
                decomposition,
                transform,
                storage,
                cancel,
            )?
        } else {
            filter_2d(
                IDWTInput::from_output(scratch_buf),
                &mut output.coefficients,
                decomposition,
                transform,
                storage,
                cancel,
            )?
        };
    }

    output.rect = temp_output.rect;

    Ok(())
}

struct IDWTInput<'a> {
    coefficients: &'a [f32],
}

impl<'a> IDWTInput<'a> {
    fn from_sub_band(sub_band: &'a SubBand, storage: &'a DecompositionStorage<'_>) -> Self {
        IDWTInput {
            coefficients: &storage.coefficients[sub_band.coefficients.clone()],
        }
    }

    fn from_output(coefficients: &'a [f32]) -> Self {
        IDWTInput { coefficients }
    }
}

fn filter_2d(
    input: IDWTInput<'_>,
    coefficients: &mut Vec<f32>,
    decomposition: &Decomposition,
    transform: WaveletTransform,
    storage: &DecompositionStorage<'_>,
    cancel: Cancel<'_>,
) -> Result<IDWTTempOutput> {
    dispatch!(Level::new(), simd => {
        filter_2d_inner(simd, input, coefficients, decomposition, transform, storage, cancel)
    })?;

    Ok(IDWTTempOutput {
        rect: decomposition.rect,
    })
}

#[inline(always)]
fn filter_2d_inner<S: Simd>(
    simd: S,
    input: IDWTInput<'_>,
    coefficients: &mut Vec<f32>,
    decomposition: &Decomposition,
    transform: WaveletTransform,
    storage: &DecompositionStorage<'_>,
    cancel: Cancel<'_>,
) -> Result<()> {
    let width = decomposition.rect.width() as usize;
    let height = decomposition.rect.height() as usize;

    assert!(coefficients.capacity() >= width * height);

    // No clearing needed: interleaving overwrites every sample.
    cancel.resize(coefficients, width * height)?;

    if width == 0 || height == 0 {
        return Ok(());
    }

    interleave_and_filter_horizontal(
        simd,
        input,
        coefficients,
        decomposition,
        transform,
        storage,
        cancel,
    )?;
    filter_vertical(simd, coefficients, decomposition.rect, transform, cancel)
}

#[inline(always)]
fn interleave_and_filter_horizontal<S: Simd>(
    simd: S,
    input: IDWTInput<'_>,
    coefficients: &mut [f32],
    decomposition: &Decomposition,
    transform: WaveletTransform,
    storage: &DecompositionStorage<'_>,
    cancel: Cancel<'_>,
) -> Result<()> {
    let IntRect {
        x0: u0,
        x1: u1,
        y0: v0,
        ..
    } = decomposition.rect;
    let width = decomposition.rect.width() as usize;

    let ll = input.coefficients;
    let [hl, lh, hh] = decomposition
        .sub_bands
        .map(|idx| &storage.coefficients[storage.sub_bands[idx].coefficients.clone()]);

    let num_low = (u1.div_ceil(2) - u0.div_ceil(2)) as usize;
    let num_high = (u1 / 2 - u0 / 2) as usize;
    let mut filter = HorizontalFilter::new(transform, u0.is_multiple_of(2), num_low, num_high);

    let (first_low_row, first_high_row) = if v0.is_multiple_of(2) { (0, 1) } else { (1, 0) };
    filter.filter_rows(
        simd,
        ll,
        hl,
        coefficients
            .chunks_exact_mut(width)
            .skip(first_low_row)
            .step_by(2),
        cancel,
    )?;
    filter.filter_rows(
        simd,
        lh,
        hh,
        coefficients
            .chunks_exact_mut(width)
            .skip(first_high_row)
            .step_by(2),
        cancel,
    )
}

struct HorizontalFilter {
    transform: WaveletTransform,
    low_first: bool,
    num_low: usize,
    num_high: usize,
    low: Vec<f32>,
    high: Vec<f32>,
}

impl HorizontalFilter {
    fn new(transform: WaveletTransform, low_first: bool, num_low: usize, num_high: usize) -> Self {
        Self {
            transform,
            low_first,
            num_low,
            num_high,
            low: vec![0.0; num_low + 2],
            high: vec![0.0; num_high + 2],
        }
    }

    #[inline(always)]
    fn filter_rows<'a, S: Simd>(
        &mut self,
        simd: S,
        low_band: &[f32],
        high_band: &[f32],
        rows: impl Iterator<Item = &'a mut [f32]>,
        cancel: Cancel<'_>,
    ) -> Result<()> {
        for (v, row) in rows.enumerate() {
            cancel.check()?;

            self.low[1..=self.num_low]
                .copy_from_slice(&low_band[v * self.num_low..][..self.num_low]);
            self.high[1..=self.num_high]
                .copy_from_slice(&high_band[v * self.num_high..][..self.num_high]);

            filter_1d(simd, self.transform, self)?;

            let low = &self.low[1..=self.num_low];
            let high = &self.high[1..=self.num_high];

            if self.low_first {
                interleave_row(simd, low, high, row);
            } else {
                interleave_row(simd, high, low, row);
            }
        }

        Ok(())
    }
}

impl<S: Simd> Lifting<S> for HorizontalFilter {
    #[inline(always)]
    fn num_samples(&self) -> usize {
        self.num_low + self.num_high
    }

    #[inline(always)]
    fn scale_low(&mut self, simd: S, factor: f32) -> Result<()> {
        scale(simd, &mut self.low[1..=self.num_low], factor);

        Ok(())
    }

    #[inline(always)]
    fn scale_high(&mut self, simd: S, factor: f32) -> Result<()> {
        scale(simd, &mut self.high[1..=self.num_high], factor);

        Ok(())
    }

    #[inline(always)]
    fn lift_low(
        &mut self,
        simd: S,
        f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
        f_scalar: impl Fn(f32, f32, f32) -> f32,
    ) -> Result<()> {
        let offset = usize::from(self.low_first);
        lift_band(
            simd,
            &mut self.low,
            &mut self.high,
            offset,
            f_simd,
            f_scalar,
        );

        Ok(())
    }

    #[inline(always)]
    fn lift_high(
        &mut self,
        simd: S,
        f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
        f_scalar: impl Fn(f32, f32, f32) -> f32,
    ) -> Result<()> {
        let offset = usize::from(!self.low_first);
        lift_band(
            simd,
            &mut self.high,
            &mut self.low,
            offset,
            f_simd,
            f_scalar,
        );

        Ok(())
    }
}

// Both bands have one sample of padding on each side.
#[inline(always)]
fn lift_band<S: Simd>(
    simd: S,
    target: &mut [f32],
    source: &mut [f32],
    offset: usize,
    f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
    f_scalar: impl Fn(f32, f32, f32) -> f32,
) {
    let last = source.len() - 1;
    source[0] = source[1];
    source[last] = source[last - 1];

    let len = target.len() - 2;
    let target = &mut target[1..][..len];
    let left = &source[1 - offset..][..len];
    let right = &source[2 - offset..][..len];

    lift_samples(simd, target, left, right, f_simd, f_scalar);
}

#[inline(always)]
fn interleave_row<S: Simd>(simd: S, first: &[f32], second: &[f32], output: &mut [f32]) {
    let num_pairs = first.len().min(second.len());
    let (first_chunks, first_rest) = first[..num_pairs].as_chunks::<SIMD_WIDTH>();
    let (second_chunks, second_rest) = second[..num_pairs].as_chunks::<SIMD_WIDTH>();
    let (output_chunks, output_rest) =
        output[..num_pairs * 2].as_chunks_mut::<{ 2 * SIMD_WIDTH }>();

    for ((f, s), out) in first_chunks.iter().zip(second_chunks).zip(output_chunks) {
        let f = f32x8::from_slice(simd, f);
        let s = f32x8::from_slice(simd, s);
        let (low, high) = out.split_at_mut(SIMD_WIDTH);
        f.zip_low(s).store(low);
        f.zip_high(s).store(high);
    }

    let (output_pairs, _) = output_rest.as_chunks_mut::<2>();
    for ((f, s), out) in first_rest.iter().zip(second_rest).zip(output_pairs) {
        *out = [*f, *s];
    }

    if first.len() > num_pairs {
        output[num_pairs * 2] = first[num_pairs];
    }
}

#[inline(always)]
fn filter_vertical<S: Simd>(
    simd: S,
    coefficients: &mut [f32],
    rect: IntRect,
    transform: WaveletTransform,
    cancel: Cancel<'_>,
) -> Result<()> {
    let mut filter = VerticalFilter {
        cancel,
        coefficients,
        width: rect.width() as usize,
        height: rect.height() as usize,
        low_first: rect.y0.is_multiple_of(2),
    };

    filter_1d(simd, transform, &mut filter)
}

struct VerticalFilter<'a> {
    cancel: Cancel<'a>,
    coefficients: &'a mut [f32],
    width: usize,
    height: usize,
    low_first: bool,
}

impl VerticalFilter<'_> {
    #[inline(always)]
    fn scale_rows<S: Simd>(&mut self, simd: S, first_row: usize, factor: f32) -> Result<()> {
        for row in self
            .coefficients
            .chunks_exact_mut(self.width)
            .skip(first_row)
            .step_by(2)
        {
            self.cancel.check()?;

            scale(simd, row, factor);
        }

        Ok(())
    }

    #[inline(always)]
    fn lift_rows<S: Simd>(
        &mut self,
        simd: S,
        first_row: usize,
        f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
        f_scalar: impl Fn(f32, f32, f32) -> f32,
    ) -> Result<()> {
        let width = self.width;
        let height = self.height;

        for row in (first_row..height).step_by(2) {
            self.cancel.check()?;

            let above = if row == 0 { 1 } else { row - 1 };
            let below = if row == height - 1 {
                height - 2
            } else {
                row + 1
            };

            let (rows_before, rest) = self.coefficients.split_at_mut(row * width);
            let (current, rows_after) = rest.split_at_mut(width);
            let neighbor = |r: usize| {
                if r < row {
                    &rows_before[r * width..][..width]
                } else {
                    &rows_after[(r - row - 1) * width..][..width]
                }
            };

            lift_samples(
                simd,
                current,
                neighbor(above),
                neighbor(below),
                &f_simd,
                &f_scalar,
            );
        }

        Ok(())
    }
}

impl<S: Simd> Lifting<S> for VerticalFilter<'_> {
    #[inline(always)]
    fn num_samples(&self) -> usize {
        self.height
    }

    #[inline(always)]
    fn scale_low(&mut self, simd: S, factor: f32) -> Result<()> {
        self.scale_rows(simd, usize::from(!self.low_first), factor)
    }

    #[inline(always)]
    fn scale_high(&mut self, simd: S, factor: f32) -> Result<()> {
        self.scale_rows(simd, usize::from(self.low_first), factor)
    }

    #[inline(always)]
    fn lift_low(
        &mut self,
        simd: S,
        f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
        f_scalar: impl Fn(f32, f32, f32) -> f32,
    ) -> Result<()> {
        self.lift_rows(simd, usize::from(!self.low_first), f_simd, f_scalar)
    }

    #[inline(always)]
    fn lift_high(
        &mut self,
        simd: S,
        f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
        f_scalar: impl Fn(f32, f32, f32) -> f32,
    ) -> Result<()> {
        self.lift_rows(simd, usize::from(self.low_first), f_simd, f_scalar)
    }
}

trait Lifting<S: Simd> {
    fn num_samples(&self) -> usize;

    fn scale_low(&mut self, simd: S, factor: f32) -> Result<()>;

    fn scale_high(&mut self, simd: S, factor: f32) -> Result<()>;

    fn lift_low(
        &mut self,
        simd: S,
        f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
        f_scalar: impl Fn(f32, f32, f32) -> f32,
    ) -> Result<()>;

    fn lift_high(
        &mut self,
        simd: S,
        f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
        f_scalar: impl Fn(f32, f32, f32) -> f32,
    ) -> Result<()>;
}

#[inline(always)]
fn filter_1d<S: Simd>(
    simd: S,
    transform: WaveletTransform,
    signal: &mut impl Lifting<S>,
) -> Result<()> {
    if signal.num_samples() == 1 {
        return signal.scale_high(simd, 0.5);
    }

    match transform {
        WaveletTransform::Reversible53 => filter_53r(simd, signal),
        WaveletTransform::Irreversible97 => filter_97i(simd, signal),
    }
}

#[inline(always)]
fn filter_53r<S: Simd>(simd: S, signal: &mut impl Lifting<S>) -> Result<()> {
    signal.lift_low(
        simd,
        #[inline(always)]
        |s, left, right| s - ((left + right + 2.0) * 0.25).floor(),
        #[inline(always)]
        |s, left, right| s - math::floor_f32(math::mul_add(left + right, 0.25, 0.5)),
    )?;

    signal.lift_high(
        simd,
        #[inline(always)]
        |s, left, right| s + ((left + right) * 0.5).floor(),
        #[inline(always)]
        |s, left, right| s + math::floor_f32((left + right) * 0.5),
    )?;

    Ok(())
}

#[inline(always)]
fn filter_97i<S: Simd>(simd: S, signal: &mut impl Lifting<S>) -> Result<()> {
    const NEG_ALPHA: f32 = 1.586_134_3;
    const NEG_BETA: f32 = 0.052_980_117;
    const NEG_GAMMA: f32 = -0.882_911_1;
    const NEG_DELTA: f32 = -0.443_506_87;
    const KAPPA: f32 = 1.230_174_1;
    const INV_KAPPA: f32 = 1.0 / KAPPA;

    let neg_alpha = f32x8::splat(simd, NEG_ALPHA);
    let neg_beta = f32x8::splat(simd, NEG_BETA);
    let neg_gamma = f32x8::splat(simd, NEG_GAMMA);
    let neg_delta = f32x8::splat(simd, NEG_DELTA);

    signal.scale_low(simd, KAPPA)?;
    signal.scale_high(simd, INV_KAPPA)?;

    signal.lift_low(
        simd,
        #[inline(always)]
        |s, left, right| (left + right).mul_add(neg_delta, s),
        #[inline(always)]
        |s, left, right| math::mul_add(left + right, NEG_DELTA, s),
    )?;

    signal.lift_high(
        simd,
        #[inline(always)]
        |s, left, right| (left + right).mul_add(neg_gamma, s),
        #[inline(always)]
        |s, left, right| math::mul_add(left + right, NEG_GAMMA, s),
    )?;

    signal.lift_low(
        simd,
        #[inline(always)]
        |s, left, right| (left + right).mul_add(neg_beta, s),
        #[inline(always)]
        |s, left, right| math::mul_add(left + right, NEG_BETA, s),
    )?;

    signal.lift_high(
        simd,
        #[inline(always)]
        |s, left, right| (left + right).mul_add(neg_alpha, s),
        #[inline(always)]
        |s, left, right| math::mul_add(left + right, NEG_ALPHA, s),
    )?;

    Ok(())
}

#[inline(always)]
fn lift_samples<S: Simd>(
    simd: S,
    target: &mut [f32],
    left: &[f32],
    right: &[f32],
    f_simd: impl Fn(f32x8<S>, f32x8<S>, f32x8<S>) -> f32x8<S>,
    f_scalar: impl Fn(f32, f32, f32) -> f32,
) {
    let (target_chunks, target_rest) = target.as_chunks_mut::<SIMD_WIDTH>();
    let (left_chunks, left_rest) = left.as_chunks::<SIMD_WIDTH>();
    let (right_chunks, right_rest) = right.as_chunks::<SIMD_WIDTH>();

    for ((t, l), r) in target_chunks.iter_mut().zip(left_chunks).zip(right_chunks) {
        let s = f32x8::from_slice(simd, t);
        let l = f32x8::from_slice(simd, l);
        let r = f32x8::from_slice(simd, r);
        f_simd(s, l, r).store(t);
    }

    for ((t, l), r) in target_rest.iter_mut().zip(left_rest).zip(right_rest) {
        *t = f_scalar(*t, *l, *r);
    }
}

#[inline(always)]
fn scale<S: Simd>(simd: S, samples: &mut [f32], factor: f32) {
    let factor_simd = f32x8::splat(simd, factor);
    let (chunks, rest) = samples.as_chunks_mut::<SIMD_WIDTH>();

    for chunk in chunks {
        (f32x8::from_slice(simd, chunk) * factor_simd).store(chunk);
    }

    for sample in rest {
        *sample *= factor;
    }
}
