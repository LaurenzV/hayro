mod image;
mod mask;

pub(crate) use image::{DecodedImage, decode_image};
pub(crate) use mask::{DecodedMask, decode_mask};

use crate::InterpreterWarning;
use crate::color::ColorSpace;
use crate::function::interpolate;
use crate::x_object::image::{ImageKind, ImageXObject};
use crate::{ImageData, LumaData};
use hayro_syntax::bit_reader::BitReader;
use hayro_syntax::object::Array;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::stream::{FilterResult, ImageColorSpace, ImageDecodeParams};
use smallvec::SmallVec;
use std::iter;

struct DecodeContext<'a> {
    decoded: FilterResult<'a>,
    width: u32,
    height: u32,
    scale_factors: (f32, f32),
    color_space: ColorSpace,
    bits_per_component: u8,
    decode_arr: SmallVec<[(f32, f32); 4]>,
}

fn decode_context<'a>(
    obj: &ImageXObject<'a>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodeContext<'a>> {
    let dict = obj.stream.dict();
    let dict_bpc = dict
        .get::<u8>(BPC)
        .or_else(|| dict.get::<u8>(BITS_PER_COMPONENT));
    let color_space = obj.color_space.clone();
    let is_indexed = obj.color_space.as_ref().is_some_and(|cs| cs.is_indexed());

    let decode_params = ImageDecodeParams {
        is_indexed,
        bpc: dict_bpc,
        num_components: color_space.as_ref().map(|c| c.num_components()),
        target_dimension,
        width: obj.width,
        height: obj.height,
    };

    let decoded = obj
        .stream
        .decoded_image(&decode_params)
        .map_err(|_| (obj.warning_sink)(InterpreterWarning::ImageDecodeFailure))
        .ok()?;

    let (mut scale_x, mut scale_y) = (1.0, 1.0);

    let (width, height) = decoded
        .image_data
        .as_ref()
        .map(|d| {
            scale_x = obj.width as f32 / d.width as f32;
            scale_y = obj.height as f32 / d.height as f32;

            (d.width, d.height)
        })
        .unwrap_or((obj.width, obj.height));

    let color_space = color_space
        .or_else(|| {
            decoded
                .image_data
                .as_ref()
                .map(|i| i.color_space)
                .and_then(|c| {
                    c.and_then(|c| match c {
                        ImageColorSpace::Gray => Some(ColorSpace::device_gray()),
                        ImageColorSpace::Rgb => Some(ColorSpace::device_rgb()),
                        ImageColorSpace::Cmyk => Some(ColorSpace::device_cmyk()),
                        ImageColorSpace::Unknown(_) => None,
                    })
                })
        })
        .unwrap_or(ColorSpace::device_gray());

    let fallback_bpc = if obj.kind == ImageKind::StencilMask {
        1
    } else {
        8
    };

    let bits_per_component = decoded
        .image_data
        .as_ref()
        .map(|i| i.bits_per_component)
        .or(dict_bpc)
        .unwrap_or(fallback_bpc);

    let decode_arr = dict
        .get::<Array<'_>>(D)
        .or_else(|| dict.get::<Array<'_>>(DECODE))
        .map(|a| a.iter::<(f32, f32)>().collect::<SmallVec<_>>())
        .unwrap_or(color_space.default_decode_arr(bits_per_component as f32));

    Some(DecodeContext {
        decoded,
        width,
        height,
        scale_factors: (scale_x, scale_y),
        color_space,
        bits_per_component,
        decode_arr,
    })
}

/// Integer box-filter downsample, in place. Only acts (reallocating `data`)
/// when the image is at least 2x `target` in both dimensions; otherwise a
/// no-op -- no reallocation, `width`/`height`/`data` left untouched.
/// `num_components` is 3 for RGB, 1 for luma/alpha.
///
/// Averages each `k`x`k` window per channel, dividing by the actual sample
/// count in that window (partial windows at the right/bottom edges when `k`
/// doesn't evenly divide the source dimensions).
pub(crate) fn downsample_to_target(
    data: &mut Vec<u8>,
    num_components: usize,
    width: &mut u32,
    height: &mut u32,
    target: (u32, u32),
) {
    let (tw, th) = target;
    let (w, h) = (*width, *height);

    if w == 0 || h == 0 || num_components == 0 {
        return;
    }

    let k = (w / (2 * tw.max(1))).min(h / (2 * th.max(1)));

    if k < 2 {
        return;
    }

    let new_w = w.div_ceil(k);
    let new_h = h.div_ceil(k);

    let mut out = vec![0_u8; new_w as usize * new_h as usize * num_components];
    // `num_components` is always 1 or 3 here (luma or RGB); 4 gives headroom.
    let mut sums = [0_u64; 4];

    for oy in 0..new_h {
        let y0 = oy * k;
        let y1 = (y0 + k).min(h);

        for ox in 0..new_w {
            let x0 = ox * k;
            let x1 = (x0 + k).min(w);

            let count = u64::from(y1 - y0) * u64::from(x1 - x0);
            if count == 0 {
                continue;
            }

            sums[..num_components].fill(0);

            for y in y0..y1 {
                let row_base = y as usize * w as usize * num_components;

                for x in x0..x1 {
                    let px_base = row_base + x as usize * num_components;

                    for (c, sum) in sums.iter_mut().enumerate().take(num_components) {
                        *sum += u64::from(data[px_base + c]);
                    }
                }
            }

            let out_base = (oy as usize * new_w as usize + ox as usize) * num_components;
            for (c, sum) in sums.iter().enumerate().take(num_components) {
                out[out_base + c] = (*sum / count) as u8;
            }
        }
    }

    *data = out;
    *width = new_w;
    *height = new_h;
}

/// Downsample a final `ImageData` (post color-space conversion, so this
/// works uniformly regardless of which syntax-level filter produced it)
/// toward `target`, recomputing `scale_factors` as `dict_dim / new_dim` --
/// the same invariant `decode_context` establishes above (`scale_x =
/// obj.width as f32 / d.width as f32`), just re-derived against the shrunk
/// dimensions instead of the originally-decoded ones.
pub(crate) fn downsample_image_data(
    image: &mut ImageData,
    dict_w: u32,
    dict_h: u32,
    target: (u32, u32),
) {
    match image {
        ImageData::Rgb(d) => {
            downsample_to_target(&mut d.data, 3, &mut d.width, &mut d.height, target);
            d.scale_factors = (
                dict_w as f32 / d.width as f32,
                dict_h as f32 / d.height as f32,
            );
        }
        ImageData::Luma(d) => {
            downsample_to_target(&mut d.data, 1, &mut d.width, &mut d.height, target);
            d.scale_factors = (
                dict_w as f32 / d.width as f32,
                dict_h as f32 / d.height as f32,
            );
        }
    }
}

/// Same treatment as `downsample_image_data`, for a standalone alpha/
/// stencil-mask `LumaData`. `dict_w`/`dict_h` are the mask's own dict
/// dimensions (which can differ from the color image it accompanies) --
/// callers derive them from whichever `ImageXObject` decoded this luma.
pub(crate) fn downsample_luma(luma: &mut LumaData, dict_w: u32, dict_h: u32, target: (u32, u32)) {
    downsample_to_target(&mut luma.data, 1, &mut luma.width, &mut luma.height, target);
    luma.scale_factors = (
        dict_w as f32 / luma.width as f32,
        dict_h as f32 / luma.height as f32,
    );
}

#[must_use]
fn fix_image_length<T: Copy>(
    image: &mut Vec<T>,
    width: u32,
    height: &mut u32,
    filler: T,
    num_components: usize,
) -> Option<()> {
    let row_len = width as usize * num_components;

    if (row_len * *height as usize) <= image.len() {
        // Too much data (or just the right amount), truncate it.
        image.truncate(row_len * *height as usize);
    } else {
        // Too little data, adapt the height and pad.
        *height = image.len().div_ceil(row_len) as u32;

        if !image.len().is_multiple_of(row_len) {
            image.extend(iter::repeat_n(filler, row_len - (image.len() % row_len)));
        }
    }

    if width == 0 || *height == 0 {
        None
    } else {
        Some(())
    }
}

fn decode_u8_samples(
    data: &[u8],
    width: u32,
    height: u32,
    color_space: &ColorSpace,
    bits_per_component: u8,
    decode: &[(f32, f32)],
) -> Option<Vec<u8>> {
    let source_max = 2.0_f32.powi(bits_per_component as i32) - 1.0;
    let num_components = color_space.num_components() as usize;
    let capacity = width as usize * height as usize * num_components;
    let ranges = color_space.component_ranges();
    let indexed_hival = color_space.indexed_hival();

    let decode_component = |value: u32, index: usize| {
        let component_index = index % num_components;
        let (decode_min, decode_max) = *decode.get(component_index)?;
        let decoded = interpolate(value as f32, 0.0, source_max, decode_min, decode_max);

        if let Some(hival) = indexed_hival {
            Some((decoded + 0.5).clamp(0.0, hival as f32) as u8)
        } else {
            let (range_min, range_max) = *ranges.get(component_index)?;
            let normalized = if range_min == range_max {
                0.0
            } else {
                (decoded - range_min) / (range_max - range_min)
            };
            Some((normalized * 255.0 + 0.5) as u8)
        }
    };

    match bits_per_component {
        1..8 | 9..16 => {
            let mut buf = Vec::with_capacity(capacity);
            for_each_sample(
                data,
                width,
                height,
                num_components,
                bits_per_component,
                |value, index| {
                    buf.push(decode_component(value, index)?);
                    Some(())
                },
            )?;

            Some(buf)
        }
        8 => Some(
            data.iter()
                .enumerate()
                .map(|(index, value)| decode_component(*value as u32, index))
                .collect::<Option<Vec<_>>>()?,
        ),
        16 => Some(
            data.chunks_exact(2)
                .enumerate()
                .map(|(index, value)| {
                    decode_component(u16::from_be_bytes([value[0], value[1]]) as u32, index)
                })
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => {
            warn!("unsupported bits per component: {bits_per_component}");
            None
        }
    }
}

fn unpack_samples(
    data: &[u8],
    width: u32,
    height: u32,
    num_components: usize,
    bits_per_component: u8,
) -> Option<Vec<u16>> {
    let capacity = width as usize * height as usize * num_components;

    match bits_per_component {
        1..8 | 9..16 => {
            let mut buf = Vec::with_capacity(capacity);
            for_each_sample(
                data,
                width,
                height,
                num_components,
                bits_per_component,
                |value, _| {
                    buf.push(value as u16);
                    Some(())
                },
            )?;

            Some(buf)
        }
        8 => Some(data.iter().map(|value| *value as u16).collect()),
        16 => Some(
            data.chunks_exact(2)
                .map(|value| u16::from_be_bytes([value[0], value[1]]))
                .collect(),
        ),
        _ => {
            warn!("unsupported bits per component: {bits_per_component}");
            None
        }
    }
}

fn for_each_sample(
    data: &[u8],
    width: u32,
    height: u32,
    num_components: usize,
    bits_per_component: u8,
    mut visit: impl FnMut(u32, usize) -> Option<()>,
) -> Option<()> {
    let mut reader = BitReader::new(data);
    let mut index = 0;

    for _ in 0..height {
        for _ in 0..width {
            for _ in 0..num_components {
                // See `stream_ccit_not_enough_data`, some images seemingly don't have
                // enough data, so we just pad with zeroes in this case.
                visit(reader.read(bits_per_component).unwrap_or(0), index)?;
                index += 1;
            }
        }

        reader.align();
    }

    Some(())
}
