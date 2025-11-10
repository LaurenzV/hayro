use crate::object::Dict;
use crate::object::dict::keys::COLOR_TRANSFORM;
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use std::io::Cursor;
use std::num::NonZeroU32;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

pub(crate) fn decode(
    data: &[u8],
    params: Dict,
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

    let mut out_colorspace = if let Some(num_components) = image_params.num_components
        && !matches!(num_components, 1 | 3 | 4)
    {
        ColorSpace::MultiBand(NonZeroU32::new(num_components as u32)?)
    } else {
        match decoder.input_colorspace().unwrap() {
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
            ColorSpace::CMYK => ColorSpace::CMYK,
            ColorSpace::YCCK => ColorSpace::YCCK,
            _ => ColorSpace::RGB,
        }
    };

    decoder.set_options(DecoderOptions::default().jpeg_set_out_colorspace(out_colorspace));
    let mut decoded = decoder.decode().ok().or_else(|| {
        let reader = Cursor::new(data);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(reader, options);
        decoder.decode_headers().ok()?;
        // It's possible that the APP14 marker is set, so that zune_jpeg will set the input colorspace
        // to a different one. So try decoding again with the different color space. This is probably
        // not the proper way to solve this, but it solves a test case.
        if matches!(out_colorspace, ColorSpace::YCCK | ColorSpace::CMYK) {
            out_colorspace = ColorSpace::RGB;
        } else {
            out_colorspace = ColorSpace::CMYK;
        }

        decoder.set_options(DecoderOptions::default().jpeg_set_out_colorspace(out_colorspace));
        decoder.decode().ok()
    })?;

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

    let image_data = ImageData {
        alpha: None,
        color_space: match out_colorspace {
            ColorSpace::RGB | ColorSpace::YCbCr => Some(ImageColorSpace::Rgb),
            ColorSpace::Luma => Some(ImageColorSpace::Gray),
            ColorSpace::YCCK | ColorSpace::CMYK => Some(ImageColorSpace::Cmyk),
            ColorSpace::MultiBand(_) => None,
            _ => None,
        },
        bits_per_component: 8,
        width: decoder.dimensions().unwrap().0 as u32,
        height: decoder.dimensions().unwrap().1 as u32,
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
    if jpeg_bytes.len() < 4 || jpeg_bytes[0..2] != [0xFF, 0xD8] {
        return None;
    }

    let mut pos = 2;
    let mut app14 = None;
    let mut components = Vec::new();

    while pos + 3 < jpeg_bytes.len() {
        if jpeg_bytes[pos] != 0xFF {
            return None;
        }

        let marker = jpeg_bytes[pos + 1];

        if marker == 0xFF {
            pos += 1;
            continue;
        }

        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 || marker == 0xDA {
            break;
        }

        if pos + 3 >= jpeg_bytes.len() {
            break;
        }

        let length = u16::from_be_bytes([jpeg_bytes[pos + 2], jpeg_bytes[pos + 3]]) as usize;

        // Extract APP14 segment
        if marker == 0xEE && pos + 2 + length <= jpeg_bytes.len() {
            let app14_data = &jpeg_bytes[pos + 4..pos + 2 + length];
            if app14_data.len() >= 12 && &app14_data[0..5] == b"Adobe" {
                app14 = Some(App14Segment {
                    _version: u16::from_be_bytes([app14_data[5], app14_data[6]]),
                    _flags0: u16::from_be_bytes([app14_data[7], app14_data[8]]),
                    _flags1: u16::from_be_bytes([app14_data[9], app14_data[10]]),
                    _color_transform: app14_data[11],
                });
            }
        }

        // Extract SOF (Start of Frame) components
        if (0xC0..=0xCF).contains(&marker)
            && marker != 0xC4
            && marker != 0xC8
            && marker != 0xCC
            && pos + 10 <= jpeg_bytes.len()
        {
            let num_components = jpeg_bytes[pos + 9] as usize;

            for i in 0..num_components {
                let comp_pos = pos + 10 + i * 3;
                if comp_pos + 2 < jpeg_bytes.len() {
                    let sampling = jpeg_bytes[comp_pos + 1];
                    components.push(JpegComponent {
                        id: jpeg_bytes[comp_pos],
                        _h_sampling: (sampling >> 4) & 0x0F,
                        _v_sampling: sampling & 0x0F,
                        _quantization_table: jpeg_bytes[comp_pos + 2],
                    });
                }
            }
        }

        pos += 2 + length;
    }

    Some(JpegData { app14, components })
}
