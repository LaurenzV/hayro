use crate::object::Dict;
use crate::object::dict::keys::COLOR_TRANSFORM;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use core::num::NonZeroU32;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::colorspace::ColorSpace::CMYK;
use zune_jpeg::zune_core::options::DecoderOptions;

pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
) -> Option<FilterResult<'static>> {
    if image_params.width > u16::MAX as u32 || image_params.height > u16::MAX as u32 {
        return None;
    }

    // Some PDFs have weird JPEGs where the JPEG metadata is completely wrong
    // (for example indicating that one of the dimensions is u16::MAX), but the
    // metadata in the PDF image dictionary is correct. Therefore, we first
    // validate the JPEG metadata and patch the data if any of the dimensions
    // are too large (if they are too small, they will just be padded later on).
    let data = maybe_patch_jpeg_dimensions(data, image_params)?;

    let options = DecoderOptions::default()
        .set_max_width(u16::MAX as usize)
        .set_max_height(u16::MAX as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(&*data), options);
    decoder.decode_headers().ok()?;

    let color_transform = params.get::<u8>(COLOR_TRANSFORM);
    let input_color_space = decoder.input_colorspace().unwrap();

    let mut out_colorspace = if let Some(num_components) = image_params.num_components
        && !matches!(num_components, 1 | 3 | 4)
    {
        ColorSpace::MultiBand(NonZeroU32::new(num_components as u32)?)
    } else {
        match input_color_space {
            ColorSpace::YCbCr => {
                if color_transform.is_none_or(|c| c == 1) {
                    ColorSpace::RGB
                } else {
                    ColorSpace::YCbCr
                }
            }
            ColorSpace::RGB | ColorSpace::RGBA => ColorSpace::RGB,
            ColorSpace::Luma | ColorSpace::LumaA => ColorSpace::Luma,
            // TODO: Find test case with color transform on cmyk
            CMYK => CMYK,
            ColorSpace::YCCK => ColorSpace::YCCK,
            _ => ColorSpace::RGB,
        }
    };

    // In case image had APP14 marker, we might have to override the colorspace.
    if input_color_space == CMYK && decoder.info().unwrap().components == 3 {
        out_colorspace = ColorSpace::RGB;
    }

    decoder.set_options(DecoderOptions::default().jpeg_set_out_colorspace(out_colorspace));
    let mut decoded = decoder.decode().ok()?;

    if out_colorspace == ColorSpace::YCCK {
        // See <https://github.com/mozilla/pdf.js/blob/69595a29192b7704733404a42a2ebb537601117b/src/core/jpg.js#L1331>
        for c in decoded.chunks_mut(4) {
            let y = c[0] as f32;
            let cb = c[1] as f32;
            let cr = c[2] as f32;
            c[0] = (434.456 - y - 1.402 * cr) as u8;
            c[1] = (119.541 - y + 0.344 * cb + 0.714 * cr) as u8;
            c[2] = (481.816 - y - 1.772 * cb) as u8;
        }
    }

    let mut width = decoder.dimensions().unwrap().0 as u32;
    let mut height = decoder.dimensions().unwrap().1 as u32;

    // A caller that wants a much smaller image (a huge scan drawn small) gets it reduced by a whole factor
    // here, in the JPEG's own colour space: the colour conversion and every copy after it then work on the
    // small image. A 10800 x 21600 CMYK JPEG is 933 MB decoded; its RGB and RGBA copies were 1.6 GB more.
    if let Some((tw, th)) = image_params.target_dimension {
        let factor = (width / tw.max(1)).min(height / th.max(1));
        let pixels = (width as usize).checked_mul(height as usize)?;
        if factor >= 2 && pixels > 0 && decoded.len() % pixels == 0 {
            let components = decoded.len() / pixels;
            decoded = box_reduce(&decoded, width, height, components, factor);
            width = width.div_ceil(factor);
            height = height.div_ceil(factor);
        }
    }

    let image_data = ImageData {
        alpha: None,
        color_space: match out_colorspace {
            ColorSpace::RGB | ColorSpace::YCbCr => Some(ImageColorSpace::Rgb),
            ColorSpace::Luma => Some(ImageColorSpace::Gray),
            ColorSpace::YCCK | CMYK => Some(ImageColorSpace::Cmyk),
            ColorSpace::MultiBand(_) => None,
            _ => None,
        },
        bits_per_component: 8,
        width,
        height,
    };

    Some(FilterResult {
        data: Cow::Owned(decoded),
        image_data: Some(image_data),
    })
}

/// Average `factor` x `factor` blocks of interleaved 8-bit samples (the last row and column of blocks may be
/// smaller).
fn box_reduce(
    data: &[u8],
    width: u32,
    height: u32,
    components: usize,
    factor: u32,
) -> Vec<u8> {
    let (w, h, f) = (width as usize, height as usize, factor as usize);
    let (ow, oh) = (w.div_ceil(f), h.div_ceil(f));
    let mut out = Vec::with_capacity(ow * oh * components);
    let mut acc = alloc::vec![0_u32; ow * components];
    for by in 0..oh {
        acc.iter_mut().for_each(|a| *a = 0);
        let rows = (by * f..((by + 1) * f).min(h)).len();
        for y in by * f..by * f + rows {
            let row = &data[y * w * components..(y + 1) * w * components];
            for (x, px) in row.chunks_exact(components).enumerate() {
                let o = (x / f) * components;
                for (a, &v) in acc[o..o + components].iter_mut().zip(px) {
                    *a += v as u32;
                }
            }
        }
        for bx in 0..ow {
            let cols = (bx * f..((bx + 1) * f).min(w)).len();
            let n = (rows * cols) as u32;
            for c in 0..components {
                out.push(((acc[bx * components + c] + n / 2) / n) as u8);
            }
        }
    }
    out
}

fn maybe_patch_jpeg_dimensions<'a>(
    data: &'a [u8],
    image_params: &ImageDecodeParams,
) -> Option<Cow<'a, [u8]>> {
    let sof_offset = find_sof_marker(data)?;

    let height_offset = sof_offset.checked_add(5)?;
    let width_offset = sof_offset.checked_add(7)?;

    let jpeg_height = u16::from_be_bytes([
        *data.get(height_offset)?,
        *data.get(height_offset.checked_add(1)?)?,
    ]);
    let jpeg_width = u16::from_be_bytes([
        *data.get(width_offset)?,
        *data.get(width_offset.checked_add(1)?)?,
    ]);

    let jpeg_area = (jpeg_width as usize).checked_mul(jpeg_height as usize)?;
    let image_area = (image_params.width as usize).checked_mul(image_params.height as usize)?;
    let need_patch = jpeg_area > image_area;

    if !need_patch {
        return Some(Cow::Borrowed(data));
    }

    let target_w = (image_params.width as u16).to_be_bytes();
    let target_h = (image_params.height as u16).to_be_bytes();

    let mut patched = data.to_vec();
    patched[height_offset..height_offset.checked_add(2)?].copy_from_slice(&target_h);
    patched[width_offset..width_offset.checked_add(2)?].copy_from_slice(&target_w);

    Some(Cow::Owned(patched))
}

fn find_sof_marker(data: &[u8]) -> Option<usize> {
    let mut i = 0_usize;

    while i.checked_add(1).is_some_and(|next| next < data.len()) {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }

        let marker = data[i + 1];

        // Note: Not sure if 100% correct/robust, is AI-generated.
        match marker {
            // All SOF markers carry dimensions: SOF0–SOF15, excluding
            // 0xC4 (DHT), 0xC8 (JPG), 0xCC (DAC) which are not frame markers.
            0xC0..=0xCF if marker != 0xC4 && marker != 0xC8 && marker != 0xCC => {
                return Some(i);
            }
            // Skip padding bytes (0xFF followed by 0xFF).
            0xFF => {
                i += 1;

                continue;
            }
            // SOI (0xD8), EOI (0xD9), TEM (0x01) and stuffed byte (0x00)
            // are standalone markers with no payload.
            0xD8 | 0xD9 | 0x01 | 0x00 => {
                i += 2;

                continue;
            }
            // All other markers have a 2-byte length field — skip over them.
            _ => {
                let len_start = i.checked_add(2)?;
                let len_end = i.checked_add(3)?;
                let seg_len =
                    u16::from_be_bytes([*data.get(len_start)?, *data.get(len_end)?]) as usize;

                i = i.checked_add(2)?.checked_add(seg_len)?;
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::box_reduce;

    #[test]
    fn box_reduce_averages_whole_and_partial_blocks() {
        // 3 x 2 pixels, 2 components; factor 2 gives 2 x 1: a 2 x 2 block and a 1 x 2 block.
        let data = [0, 100, 10, 100, 50, 0, 20, 100, 30, 100, 70, 0];
        assert_eq!(box_reduce(&data, 3, 2, 2, 2), [15, 100, 60, 0]);
    }
}
