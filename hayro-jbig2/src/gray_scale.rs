use alloc::vec::Vec;

use crate::DecodeState;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::{Bitmap, MAX_DIMENSION};
use crate::cancel::{Cancel, row_mask};
use crate::decode::generic::{ContextGatherer, decode_bitmap_mmr};
use crate::decode::{AdaptiveTemplatePixel, Template};
use crate::error::{OverflowError, ParseError, Result, bail};
use crate::memory::MemoryBudget;
use crate::simd::{self, Level, Simd, u32x8};

#[derive(Debug, Clone)]
pub(crate) struct GrayScaleParams<'a> {
    pub(crate) use_mmr: bool,
    pub(crate) bits_per_pixel: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) template: Template,
    pub(crate) skip_mask: Option<&'a [u32]>,
    pub(crate) strict: bool,
}

#[inline(always)]
pub(crate) fn decode_gray_scale_image(
    data: &[u8],
    params: &GrayScaleParams<'_>,
    ctx: &mut DecodeState<'_>,
) -> Result<GrayScaleImage> {
    if params.use_mmr {
        decode_mmr(data, params, &mut ctx.budget, ctx.cancel)
    } else {
        decode_arithmetic(data, params, ctx)
    }
}

fn decode_mmr(
    data: &[u8],
    params: &GrayScaleParams<'_>,
    budget: &mut MemoryBudget,
    cancel: Cancel<'_>,
) -> Result<GrayScaleImage> {
    let width = params.width;
    let height = params.height;
    let bits_per_pixel = params.bits_per_pixel;

    let mut offset = 0;
    decode_bitplanes(
        width,
        height,
        bits_per_pixel,
        budget,
        cancel,
        |_, bitplane| {
            offset += decode_bitmap_mmr(bitplane, &data[offset..], params.strict, cancel)?;

            Ok(())
        },
    )
}

fn decode_arithmetic(
    data: &[u8],
    params: &GrayScaleParams<'_>,
    ctx: &mut DecodeState<'_>,
) -> Result<GrayScaleImage> {
    let width = params.width;
    let height = params.height;

    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        bail!(OverflowError::BitmapDimension);
    }

    let bits_per_pixel = params.bits_per_pixel;
    let skip_mask = params.skip_mask;
    let skip_stride = width.div_ceil(32);
    let template = params.template;

    let at_pixels: [AdaptiveTemplatePixel; 4] = match template {
        Template::Template0 => [
            AdaptiveTemplatePixel { x: 3, y: -1 },
            AdaptiveTemplatePixel { x: -3, y: -1 },
            AdaptiveTemplatePixel { x: 2, y: -2 },
            AdaptiveTemplatePixel { x: -2, y: -2 },
        ],
        Template::Template1 => [
            AdaptiveTemplatePixel { x: 3, y: -1 },
            AdaptiveTemplatePixel::default(),
            AdaptiveTemplatePixel::default(),
            AdaptiveTemplatePixel::default(),
        ],
        Template::Template2 | Template::Template3 => [
            AdaptiveTemplatePixel { x: 2, y: -1 },
            AdaptiveTemplatePixel::default(),
            AdaptiveTemplatePixel::default(),
            AdaptiveTemplatePixel::default(),
        ],
    };

    let mut decoder = ArithmeticDecoder::new(data);
    let cancel = ctx.cancel;
    let check_mask = row_mask(width);
    ctx.contexts.clear();
    ctx.contexts.resize(
        1 << template.context_bits(),
        ArithmeticDecoderContext::default(),
    );

    macro_rules! gs_decode_loop {
        ($gather:expr) => {
            decode_bitplanes(
                width,
                height,
                bits_per_pixel,
                &mut ctx.budget,
                cancel,
                |_, bitplane| {
                    let mut gatherer = ContextGatherer::new(template, &at_pixels);

                    for y in 0..height {
                        if decoder.is_exhausted() {
                            bail!(ParseError::UnexpectedEof);
                        }

                        if y & check_mask == check_mask {
                            cancel.check()?;
                        }

                        gatherer.start_row(bitplane, y);
                        for x in 0..width {
                            gatherer.maybe_reload_buffers(bitplane, x);

                            if let Some(mask) = skip_mask {
                                let word_idx = (y * skip_stride + x / 32) as usize;
                                let bit_pos = 31 - (x % 32);
                                if (mask[word_idx] >> bit_pos) & 1 != 0 {
                                    // Still need to update the context.
                                    let _ = ($gather)(&mut gatherer, bitplane, x);

                                    continue;
                                }
                            }

                            let context = ($gather)(&mut gatherer, bitplane, x);
                            let pixel = decoder.read_bit(&mut ctx.contexts[context as usize]);
                            let value = pixel as u8;

                            if value != 0 {
                                bitplane.set_pixel(x, y, value);
                                gatherer.update_current_row(x, value);
                            }
                        }
                    }

                    Ok(())
                },
            )
        };
    }

    match template {
        Template::Template0 => gs_decode_loop!(ContextGatherer::gather_template0_default),
        Template::Template1 => gs_decode_loop!(ContextGatherer::gather_template1_default),
        Template::Template2 => gs_decode_loop!(ContextGatherer::gather_template2_default),
        Template::Template3 => gs_decode_loop!(ContextGatherer::gather_template3_default),
    }
}

pub(crate) struct GrayScaleImage {
    width: u32,
    planes: Vec<Bitmap>,
}

impl GrayScaleImage {
    pub(crate) fn row_values(&self, y: u32, row: &mut [u32]) {
        row.fill(0);

        simd::dispatch!(Level::new(), simd => {
            for (j, plane) in self.planes.iter().enumerate() {
                let start = (y * plane.stride) as usize;
                let data = &plane.data[start..start + plane.stride as usize];
                extract_bits(simd, data, row, self.width, 1, plane.stride, j as u32);
            }
        });
    }
}

#[inline(never)]
fn decode_bitplanes<F>(
    width: u32,
    height: u32,
    bits_per_pixel: u32,
    budget: &mut MemoryBudget,
    cancel: Cancel<'_>,
    mut decode_next: F,
) -> Result<GrayScaleImage>
where
    F: FnMut(u32, &mut Bitmap) -> Result<()>,
{
    let mut planes: Vec<Bitmap> = Vec::with_capacity(bits_per_pixel as usize);

    simd::dispatch!(Level::new(), simd => {
        for j in (0..bits_per_pixel).rev() {
            cancel.check()?;
            let mut bitplane = Bitmap::new(width, height, budget)?;
            decode_next(j, &mut bitplane)?;

            if let Some(previous) = planes.last() {
                xor_planes(simd, &mut bitplane.data, &previous.data);
            }

            planes.push(bitplane);
        }

        planes.reverse();

        Ok(GrayScaleImage { width, planes })
    })
}

#[inline(always)]
fn xor_planes<S: Simd>(simd: S, plane: &mut [u32], prev: &[u32]) {
    let simd_end = plane.len() & !7;

    for i in (0..simd_end).step_by(simd::SIMD_WIDTH) {
        let chunk = &mut plane[i..i + simd::SIMD_WIDTH];
        let mut a = u32x8::from_slice(simd, chunk);
        let b = u32x8::from_slice(simd, &prev[i..i + simd::SIMD_WIDTH]);
        a ^= b;
        a.store(chunk);
    }

    for i in simd_end..plane.len() {
        plane[i] ^= prev[i];
    }
}

#[inline(always)]
fn extract_bits<S: Simd>(
    simd: S,
    data: &[u32],
    values: &mut [u32],
    width: u32,
    height: u32,
    stride: u32,
    j: u32,
) {
    const BIT_MASKS: [u32; 32] = {
        let mut masks = [0_u32; 32];
        let mut i = 0;
        while i < 32 {
            masks[i] = 1 << (31 - i);
            i += 1;
        }
        masks
    };

    let bit_value = u32x8::splat(simd, 1_u32 << j);
    let zero = u32x8::splat(simd, 0);
    let simd_end = (width as usize) & !7;

    for y in 0..height as usize {
        let row_data = y * stride as usize;
        let row_vals = y * width as usize;

        for x in (0..simd_end).step_by(simd::SIMD_WIDTH) {
            let word = data[row_data + x / 32];
            let bit_offset = x % 32;

            let splatted = u32x8::splat(simd, word);
            let masks =
                u32x8::from_slice(simd, &BIT_MASKS[bit_offset..bit_offset + simd::SIMD_WIDTH]);
            let extracted = splatted & masks;
            let nonzero = extracted.simd_gt(zero);
            let contribution = nonzero.select(bit_value, zero);
            let vals = &mut values[row_vals + x..row_vals + x + simd::SIMD_WIDTH];
            let current = u32x8::from_slice(simd, vals);
            (current | contribution).store(vals);
        }

        for x in simd_end..width as usize {
            let word_idx = row_data + x / 32;
            let bit_pos = 31 - (x % 32);
            if (data[word_idx] >> bit_pos) & 1 != 0 {
                values[row_vals + x] |= 1 << j;
            }
        }
    }
}
