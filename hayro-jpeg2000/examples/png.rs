//! This example shows you how you can convert a JPEG2000 image into PNG using
//! the `image` crate.

use hayro_jpeg2000::{DecodeSettings, Image};
use image::{ColorType, DynamicImage, ImageBuffer, ImageDecoder};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
    let color_type = image.color_type();
    let width = image.width();
    let height = image.height();
    let mut buf = vec![0_u8; image.total_bytes() as usize];
    image.read_image(&mut buf).unwrap();

    let rgba = match color_type {
        ColorType::L8 => {
            DynamicImage::ImageLuma8(ImageBuffer::from_raw(width, height, buf).unwrap())
        }
        ColorType::La8 => {
            DynamicImage::ImageLumaA8(ImageBuffer::from_raw(width, height, buf).unwrap())
        }
        ColorType::Rgb8 => {
            DynamicImage::ImageRgb8(ImageBuffer::from_raw(width, height, buf).unwrap())
        }
        ColorType::Rgba8 => {
            DynamicImage::ImageRgba8(ImageBuffer::from_raw(width, height, buf).unwrap())
        }
        _ => unimplemented!(),
    };

    Ok(rgba)
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
