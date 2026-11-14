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

    // Some JPEGs have bogus oversized dimensions, so patch them from the image dictionary.
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

    let width = decoder.dimensions().unwrap().0 as u32;
    let height = decoder.dimensions().unwrap().1 as u32;

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

        match marker {
            // SOF0–SOF15, excluding DHT, JPG and DAC.
            0xC0..=0xCF if marker != 0xC4 && marker != 0xC8 && marker != 0xCC => {
                return Some(i);
            }
            0xFF => {
                i += 1;

                continue;
            }
            0xD8 | 0xD9 | 0x01 | 0x00 => {
                i += 2;

                continue;
            }
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
