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

    #[cfg(feature = "jpeg-decoder")]
    if let Some(scaled) = try_scaled_decode(&data, params, image_params) {
        return Some(scaled);
    }

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

/// Attempt an IDCT-scaled decode via `jpeg-decoder`, honoring
/// `image_params.target_dimension`, to avoid paying full-resolution decode
/// time and memory for an image that the renderer will only ever composite
/// at a much smaller size.
///
/// Returns `None` (never a wrong answer) on any error or unmet
/// precondition; the caller then falls through to the existing
/// full-resolution `zune-jpeg` path.
#[cfg(feature = "jpeg-decoder")]
fn try_scaled_decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
) -> Option<FilterResult<'static>> {
    let (target_w, target_h) = image_params.target_dimension?;
    if target_w < 1 || target_h < 1 {
        return None;
    }

    // `jpeg_decoder::Decoder::scale` takes `u16` dimensions; bail (rather
    // than truncate) if the hint is somehow outside that range.
    if target_w > u16::MAX as u32 || target_h > u16::MAX as u32 {
        return None;
    }

    // Conservative: only bother if we can at least halve both dimensions.
    // Safe from overflow: both operands are bounded by u16::MAX above.
    if image_params.width < 2 * target_w || image_params.height < 2 * target_h {
        return None;
    }

    // `jpeg-decoder` doesn't apply a PDF /DecodeParms ColorTransform
    // override; skip when one is present rather than risk diverging from
    // zune-jpeg's color handling.
    if params.get::<u8>(COLOR_TRANSFORM).is_some() {
        return None;
    }

    // The `MultiBand` override path (num_components outside 1/3, e.g. spot
    // colors) isn't handled by the scaled decoder; fall through for those.
    if let Some(n) = image_params.num_components
        && !matches!(n, 1 | 3)
    {
        return None;
    }

    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(data));
    decoder.read_info().ok()?;
    let info = decoder.info()?;

    // Only the dominant scanned-document cases: grayscale and YCbCr/RGB.
    // CMYK32 and L16 fall through to zune-jpeg.
    let color_space = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => ImageColorSpace::Gray,
        jpeg_decoder::PixelFormat::RGB24 => ImageColorSpace::Rgb,
        _ => return None,
    };

    let req_w = target_w.min(u16::MAX as u32) as u16;
    let req_h = target_h.min(u16::MAX as u32) as u16;
    let (actual_w, actual_h) = decoder.scale(req_w, req_h).ok()?;
    let pixels = decoder.decode().ok()?;

    let image_data = ImageData {
        alpha: None,
        color_space: Some(color_space),
        bits_per_component: 8,
        width: actual_w as u32,
        height: actual_h as u32,
    };

    Some(FilterResult {
        data: Cow::Owned(pixels),
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
