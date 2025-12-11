use crate::object::Dict;
use crate::object::dict::keys::COLOR_TRANSFORM;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use std::io::Cursor;
use std::num::NonZeroU32;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::colorspace::ColorSpace::CMYK;
use zune_jpeg::zune_core::options::DecoderOptions;

pub(crate) fn decode(
    data: &[u8],
    params: Dict<'_>,
    image_params: &ImageDecodeParams,
) -> Option<FilterResult> {
    let reader = Cursor::new(data);
    let options = DecoderOptions::default()
        .set_max_width(u16::MAX as usize)
        .set_max_height(u16::MAX as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(reader, options);
    decoder.decode_headers().ok()?;

    let jpeg_data = extract_jpeg_data(data)?;

    let color_transform = params.get::<u8>(COLOR_TRANSFORM);
    let input_color_space = decoder.input_colorspace().unwrap();

    let mut out_colorspace = if let Some(num_components) = image_params.num_components
        && !matches!(num_components, 1 | 3 | 4)
    {
        ColorSpace::MultiBand(NonZeroU32::new(num_components as u32)?)
    } else {
        match input_color_space {
            ColorSpace::YCbCr => {
                if jpeg_data.app14.is_none()
                    && jpeg_data.components.first()?.id == b'R'
                    && jpeg_data.components.get(1)?.id == b'G'
                    && jpeg_data.components.get(2)?.id == b'B'
                {
                    // pdf.js issue 11931, actual image data is RGB but zune-jpeg seems to register
                    // YCbCr, so choose YCbCr to prevent zune-jpeg from applying the transform.
                    ColorSpace::YCbCr
                } else if color_transform.is_none_or(|c| c == 1) {
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

    let expected_len = out_colorspace.num_components()
        * image_params.width as usize
        * image_params.height as usize;

    // If actual image is larger than expected, truncate data and treat the
    // PDF metadata as authoritative. If actual image is smaller than the PDF
    // metadata, treat the JPEG metadata as authoritative.
    if expected_len < decoded.len() {
        decoded.truncate(expected_len);
        width = image_params.width;
        height = image_params.height;
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
        data: decoded,
        image_data: Some(image_data),
    })
}

#[derive(Debug)]
struct App14Segment {
    _version: u16,
    _flags0: u16,
    _flags1: u16,
    _color_transform: u8,
}

#[derive(Debug)]
struct JpegComponent {
    id: u8,
    _h_sampling: u8,
    _v_sampling: u8,
    _quantization_table: u8,
}

#[derive(Debug)]
struct JpegData {
    app14: Option<App14Segment>,
    components: Vec<JpegComponent>,
}

fn extract_jpeg_data(jpeg_bytes: &[u8]) -> Option<JpegData> {
    let mut data = jpeg_bytes.strip_prefix(&[0xFF, 0xD8])?;
    let mut app14 = None;
    let mut components = Vec::new();

    while let [0xFF, marker, rest @ ..] = data {
        let marker = *marker;

        // Skip padding bytes.
        if marker == 0xFF {
            data = rest;
            continue;
        }

        // Stop at restart markers, TEM, or SOS.
        if matches!(marker, 0xD0..=0xD7 | 0x01 | 0xDA) {
            break;
        }

        let [len_hi, len_lo, rest @ ..] = rest else {
            break;
        };
        let length = u16::from_be_bytes([*len_hi, *len_lo]) as usize;
        let segment = rest.get(..length.saturating_sub(2))?;

        // Extract APP14 (Adobe) segment.
        if marker == 0xEE
            && let Some(adobe_data) = segment.strip_prefix(b"Adobe")
            && let [v0, v1, f0_0, f0_1, f1_0, f1_1, color_transform, ..] = adobe_data
        {
            app14 = Some(App14Segment {
                _version: u16::from_be_bytes([*v0, *v1]),
                _flags0: u16::from_be_bytes([*f0_0, *f0_1]),
                _flags1: u16::from_be_bytes([*f1_0, *f1_1]),
                _color_transform: *color_transform,
            });
        }

        // Extract SOF (Start of Frame) components.
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
            && let [_, _, _, _, _, num_components, comp_data @ ..] = segment
        {
            for chunk in comp_data.chunks_exact(3).take(*num_components as usize) {
                let [id, sampling, quant_table] = chunk else {
                    unreachable!()
                };
                components.push(JpegComponent {
                    id: *id,
                    _h_sampling: (sampling >> 4) & 0x0F,
                    _v_sampling: sampling & 0x0F,
                    _quantization_table: *quant_table,
                });
            }
        }

        data = rest.get(length.saturating_sub(2)..)?;
    }

    Some(JpegData { app14, components })
}
