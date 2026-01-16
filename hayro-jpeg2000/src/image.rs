use crate::{ColorSpace, Image};
use ::image::error::{DecodingError, ImageFormatHint};
use ::image::{ColorType, ImageDecoder, ImageError, ImageResult};
use image::ExtendedColorType;
use moxcms::{ColorProfile, Layout, TransformOptions};

const CMYK_PROFILE: &[u8] = include_bytes!("../assets/CGATS001Compat-v2-micro.icc");

impl ImageDecoder for Image<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (self.width(), self.height())
    }

    fn color_type(&self) -> ColorType {
        let channel_count = self.color_space.num_channels();
        let has_alpha = self.has_alpha;

        match (channel_count, has_alpha) {
            (1, false) => ColorType::L8,
            (1, true) => ColorType::La8,
            (3, false) => ColorType::Rgb8,
            (3, true) => ColorType::Rgba8,
            // We convert CMYK to RGB.
            (4, false) => ColorType::Rgb8,
            (4, true) => ColorType::Rgba8,
            // We have to return something...
            _ => ColorType::Rgb8,
        }
    }

    fn original_color_type(&self) -> ExtendedColorType {
        let channel_count = self.color_space.num_channels();
        let has_alpha = self.has_alpha;
        let depth = self.original_bit_depth();
        // match logic based on color_type() above
        match (channel_count, depth, has_alpha) {
            // Grayscale
            (1, 1, false) => ExtendedColorType::L1,
            (1, 1, true) => ExtendedColorType::La1,
            (1, 2, false) => ExtendedColorType::L2,
            (1, 2, true) => ExtendedColorType::La2,
            (1, 4, false) => ExtendedColorType::L4,
            (1, 4, true) => ExtendedColorType::La4,
            (1, 8, false) => ExtendedColorType::L8,
            (1, 8, true) => ExtendedColorType::La8,
            (1, 16, false) => ExtendedColorType::L8,
            (1, 16, true) => ExtendedColorType::La8,
            // RGB
            (3, 1, false) => ExtendedColorType::Rgb1,
            (3, 1, true) => ExtendedColorType::Rgba1,
            (3, 2, false) => ExtendedColorType::Rgb2,
            (3, 2, true) => ExtendedColorType::Rgba2,
            (3, 4, false) => ExtendedColorType::Rgb4,
            (3, 4, true) => ExtendedColorType::Rgba4,
            (3, 8, false) => ExtendedColorType::Rgb8,
            (3, 8, true) => ExtendedColorType::Rgba8,
            (3, 16, false) => ExtendedColorType::Rgb8,
            (3, 16, true) => ExtendedColorType::Rgba8,
            // CMYK
            (4, 8, false) => ExtendedColorType::Cmyk8,
            (4, 16, false) => ExtendedColorType::Cmyk16,
            // CMYK with alpha is not representable
            _ => ExtendedColorType::Unknown(orig_bits_per_pixel(self)),
        }
    }

    fn read_image(self, buf: &mut [u8]) -> ImageResult<()>
    where
        Self: Sized,
    {
        convert_inner(&self, buf).ok_or(ImageError::Decoding(DecodingError::new(
            ImageFormatHint::Name("JPEG2000".to_string()),
            "failed to decode image",
        )))
    }

    fn read_image_boxed(self: Box<Self>, buf: &mut [u8]) -> ImageResult<()> {
        convert_inner(&self, buf).ok_or(ImageError::Decoding(DecodingError::new(
            ImageFormatHint::Name("JPEG2000".to_string()),
            "failed to decode image",
        )))
    }
}

/// Private convenience function for `image` integration
fn orig_bits_per_pixel(img: &Image<'_>) -> u8 {
    let mut channel_count = img.color_space().num_channels();
    if img.has_alpha {
        channel_count += 1;
    }
    channel_count * img.original_bit_depth()
}

fn convert_inner(image: &Image<'_>, buf: &mut [u8]) -> Option<()> {
    let width = image.width();
    let height = image.height();
    let color_space = image.color_space().clone();
    let has_alpha = image.has_alpha();

    fn from_icc(
        icc: &[u8],
        num_channels: u8,
        has_alpha: bool,
        width: u32,
        height: u32,
        input_data: &[u8],
    ) -> Option<Vec<u8>> {
        let src_profile = ColorProfile::new_from_slice(icc).ok()?;
        let dest_profile = ColorProfile::new_srgb();

        let (src_layout, dest_layout, out_channels) = match (num_channels, has_alpha) {
            (1, false) => (Layout::Gray, Layout::Gray, 1),
            (1, true) => (Layout::GrayAlpha, Layout::GrayAlpha, 2),
            (3, false) => (Layout::Rgb, Layout::Rgb, 3),
            (3, true) => (Layout::Rgba, Layout::Rgba, 4),
            // CMYK will be converted to RGB.
            (4, false) => (Layout::Rgba, Layout::Rgb, 3),
            _ => {
                unimplemented!()
            }
        };

        let transform = src_profile
            .create_transform_8bit(
                src_layout,
                &dest_profile,
                dest_layout,
                TransformOptions::default(),
            )
            .ok()?;

        let mut transformed = vec![0; (width * height * out_channels) as usize];

        transform.transform(input_data, &mut transformed).ok()?;

        Some(transformed)
    }

    fn process(
        image: &Image<'_>,
        buf: &mut [u8],
        width: u32,
        height: u32,
        has_alpha: bool,
        cs: ColorSpace,
    ) -> Option<()> {
        match (cs, has_alpha) {
            (ColorSpace::Gray, false) => {
                image.decode_into(buf).ok()?;
            }
            (ColorSpace::Gray, true) => {
                image.decode_into(buf).ok()?;
            }
            (ColorSpace::RGB, false) => {
                image.decode_into(buf).ok()?;
            }
            (ColorSpace::RGB, true) => {
                image.decode_into(buf).ok()?;
            }
            (ColorSpace::CMYK, false) => {
                let decoded = image.decode().ok()?;
                let transformed = from_icc(CMYK_PROFILE, 4, has_alpha, width, height, &decoded)?;
                buf.copy_from_slice(&transformed);
            }
            (ColorSpace::CMYK, true) => {
                // moxcms doesn't support CMYK interleaved with alpha, so we
                // need to split it.
                let decoded = image.decode().ok()?;
                let mut cmyk = vec![];
                let mut alpha = vec![];

                for sample in decoded.chunks_exact(5) {
                    cmyk.extend_from_slice(&sample[..4]);
                    alpha.push(sample[4]);
                }

                let rgb = from_icc(CMYK_PROFILE, 4, false, width, height, &cmyk)?;
                for (out, pixel) in buf.chunks_exact_mut(4).zip(
                    rgb.chunks_exact(3)
                        .zip(alpha)
                        .map(|(rgb, alpha)| [rgb[0], rgb[1], rgb[2], alpha]),
                ) {
                    out.copy_from_slice(&pixel);
                }
            }
            (
                ColorSpace::Icc {
                    profile,
                    num_channels: num_components,
                },
                has_alpha,
            ) => {
                let decoded = image.decode().ok()?;

                let transformed =
                    from_icc(&profile, num_components, has_alpha, width, height, &decoded);

                if let Some(transformed) = transformed {
                    buf.copy_from_slice(&transformed);
                } else {
                    match num_components {
                        1 => process(image, buf, width, height, has_alpha, ColorSpace::Gray)?,
                        3 => process(image, buf, width, height, has_alpha, ColorSpace::RGB)?,
                        4 => process(image, buf, width, height, has_alpha, ColorSpace::CMYK)?,
                        _ => return None,
                    }
                };
            }
            (ColorSpace::Unknown { .. }, _) => return None,
        };

        Some(())
    }

    process(image, buf, width, height, has_alpha, color_space)
}
