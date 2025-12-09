//! This example shows you how you can convert a JPEG2000 image into PNG using
//! the `image` crate.

use hayro_jpeg2000::{ColorSpace, DecodeSettings, Image};
use image::{DynamicImage, ImageBuffer};
use moxcms::{ColorProfile, Layout, TransformOptions};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const CMYK_PROFILE: &[u8] = include_bytes!("../assets/CGATS001Compat-v2-micro.icc");

fn main() {
    if let Ok(()) = log::set_logger(&LOGGER) {
        log::set_max_level(log::LevelFilter::Warn);
    }

    let target = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test.jp2"));

    let image = convert(&target).unwrap();
    image.save("out.png").unwrap();
}

fn convert(path: &Path) -> Result<DynamicImage, String> {
    let data = fs::read(path).unwrap();

    // The default decode settings should work for most cases.
    let settings = DecodeSettings::default();

    // Read image and its metadata.
    let image = Image::new(&data, &settings)?;
    let width = image.width();
    let height = image.height();
    let color_space = image.color_space().clone();
    let has_alpha = image.has_alpha();
    // Decode the image.
    let decoded = image.decode()?;

    fn from_icc(
        icc: &[u8],
        num_channels: u8,
        has_alpha: bool,
        width: u32,
        height: u32,
        input_data: &[u8],
    ) -> Result<DynamicImage, String> {
        let src_profile = ColorProfile::new_from_slice(icc)
            .map_err(|_| "failed to read ICC profile".to_string())?;
        let dest_profile = ColorProfile::new_srgb();

        let src_layout = match num_channels {
            1 => Layout::Gray,
            2 => Layout::GrayAlpha,
            3 => Layout::Rgb,
            4 => Layout::Rgba,
            5 => Layout::Inks5,
            _ => unimplemented!(),
        };

        let out_channels = if has_alpha { 4 } else { 3 };

        let transform = src_profile
            .create_transform_8bit(
                src_layout,
                &dest_profile,
                if has_alpha { Layout::Rgba } else { Layout::Rgb },
                TransformOptions::default(),
            )
            .unwrap();

        let mut transformed = vec![0; (width * height * out_channels) as usize];

        transform.transform(input_data, &mut transformed).unwrap();

        let image = if has_alpha {
            DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(width, height, transformed)
                    .ok_or_else(|| "failed to build rgba buffer".to_string())?,
            )
        } else {
            DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(width, height, transformed)
                    .ok_or_else(|| "failed to build rgb buffer".to_string())?,
            )
        };

        Ok(image)
    }

    fn convert(
        data: Vec<u8>,
        width: u32,
        height: u32,
        has_alpha: bool,
        cs: ColorSpace,
    ) -> Result<DynamicImage, String> {
        let image = match (cs, has_alpha) {
            (ColorSpace::Gray, false) => DynamicImage::ImageLuma8(
                ImageBuffer::from_raw(width, height, data)
                    .ok_or_else(|| "failed to build grayscale buffer".to_string())?,
            ),
            (ColorSpace::Gray, true) => DynamicImage::ImageLumaA8(
                ImageBuffer::from_raw(width, height, data)
                    .ok_or_else(|| "failed to build grayscale-alpha buffer".to_string())?,
            ),
            (ColorSpace::RGB, false) => DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(width, height, data)
                    .ok_or_else(|| "failed to build rgb buffer".to_string())?,
            ),
            (ColorSpace::RGB, true) => DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(width, height, data)
                    .ok_or_else(|| "failed to build rgba buffer".to_string())?,
            ),
            (ColorSpace::CMYK, false) => {
                from_icc(CMYK_PROFILE, 4, has_alpha, width, height, &data)?
            }
            (ColorSpace::CMYK, true) => {
                // moxcms doesn't support CMYK interleaved with alpha, so we
                // need to split it.
                let mut cmyk = vec![];
                let mut alpha = vec![];

                for sample in data.chunks_exact(5) {
                    cmyk.extend_from_slice(&sample[..4]);
                    alpha.push(sample[4]);
                }

                let rgb = from_icc(CMYK_PROFILE, 4, false, width, height, &cmyk)?;
                let interleaved = rgb
                    .as_bytes()
                    .chunks_exact(3)
                    .zip(alpha)
                    .flat_map(|(rgb, alpha)| [rgb[0], rgb[1], rgb[2], alpha])
                    .collect::<Vec<_>>();

                DynamicImage::ImageRgba8(
                    ImageBuffer::from_raw(width, height, interleaved)
                        .ok_or_else(|| "failed to build rgba buffer".to_string())?,
                )
            }
            (
                ColorSpace::Icc {
                    profile,
                    num_channels: mut num_components,
                },
                has_alpha,
            ) => {
                if has_alpha {
                    num_components += 1;
                }

                from_icc(&profile, num_components, has_alpha, width, height, &data).or_else(
                    |e| match num_components {
                        1 => convert(data, width, height, has_alpha, ColorSpace::Gray),
                        3 => convert(data, width, height, has_alpha, ColorSpace::RGB),
                        4 => convert(data, width, height, has_alpha, ColorSpace::CMYK),
                        _ => Err(e),
                    },
                )?
            }
        };

        Ok(image)
    }

    convert(decoded, width, height, has_alpha, color_space)
}

static LOGGER: SimpleLogger = SimpleLogger;

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::LevelFilter::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        eprintln!("{}", record.args());
    }

    fn flush(&self) {}
}
