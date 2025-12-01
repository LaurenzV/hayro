use hayro_jpeg2000::bitmap::Bitmap;
use hayro_jpeg2000::{ColourSpecificationMethod, DecodeSettings, EnumeratedColourspace, read};
use image::{DynamicImage, ImageBuffer};
use moxcms::{ColorProfile, Layout, TransformOptions};
use std::env;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

const ROMM_PROFILE: &[u8] = include_bytes!("../assets/ISO22028-2_ROMM-RGB.icc");

fn main() {
    if let Ok(()) = log::set_logger(&LOGGER) {
        log::set_max_level(log::LevelFilter::Warn);
    }

    let target = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test.jp2"));

    let mut inputs = collect_inputs(&target);
    if inputs.is_empty() {
        eprintln!("No JP2 files found at {}", target.to_string_lossy());
        return;
    }

    inputs.sort();

    for path in inputs {
        let result = catch_unwind(AssertUnwindSafe(|| convert_jp2(&path)));

        match result {
            Ok(conversion) => match conversion {
                Ok(_output_path) => {
                    // println!("  Wrote {}", output_path.to_string_lossy());
                }
                Err(err) => {
                    eprintln!("  Failed: {}", err);
                }
            },
            Err(_) => {
                eprintln!("  Failed: decoder panicked");
            }
        }
    }
}

fn collect_inputs(target: &Path) -> Vec<PathBuf> {
    if target.is_file() {
        if target
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("jp2"))
            .unwrap_or(false)
        {
            vec![target.to_path_buf()]
        } else {
            Vec::new()
        }
    } else if target.is_dir() {
        match fs::read_dir(target) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path
                            .extension()
                            .map(|ext| ext.eq_ignore_ascii_case("jp2"))
                            .unwrap_or(false)
                })
                .collect(),
            Err(err) => {
                eprintln!(
                    "Failed to read directory {}: {}",
                    target.to_string_lossy(),
                    err
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    }
}

fn convert_jp2(path: &Path) -> Result<PathBuf, String> {
    let data = fs::read(path).map_err(|err| format!("read error: {err}"))?;

    let settings = DecodeSettings {
        resolve_palette_indices: true,
        strict: true,
    };

    let bitmap = read(&data, &settings).map_err(|err| format!("decode error: {err}"))?;
    let dynamic = to_dynamic_image(bitmap)?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "invalid file name".to_string())?;

    let hayro_name = format!("{stem}_hayro.png");
    let output_path = path.with_file_name(hayro_name);

    dynamic
        .save(&output_path)
        .map_err(|err| format!("write error: {err}"))?;

    Ok(output_path)
}

fn to_dynamic_image(bitmap: Bitmap) -> Result<DynamicImage, String> {
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

    let (width, height) = (bitmap.metadata.width, bitmap.metadata.height);
    let mut has_alpha = bitmap.channels.iter().any(|c| c.is_alpha);
    let num_channels = bitmap.channels.len();

    if let Some(expected_channels) = bitmap
        .metadata
        .colour_specification
        .as_ref()
        .and_then(|c| c.method.expected_number_of_channels())
        && (expected_channels as usize) < num_channels
    {
        has_alpha = true;
    }

    let channels = bitmap
        .channels
        .into_iter()
        .map(|c| c.into_8bit())
        .collect::<Vec<_>>();

    let interleaved = if num_channels == 1 {
        channels[0].clone()
    } else {
        let mut interleaved = Vec::new();
        let num_samples = channels.iter().map(|c| c.len()).min().unwrap_or(0);

        for sample_idx in 0..num_samples {
            for channel in &channels {
                interleaved.push(channel[sample_idx]);
            }
        }

        interleaved
    };

    if let Some(spec) = &bitmap.metadata.colour_specification {
        match &spec.method {
            ColourSpecificationMethod::IccProfile(icc) => {
                let res = from_icc(
                    icc.as_slice(),
                    num_channels as u8,
                    has_alpha,
                    width,
                    height,
                    &interleaved,
                );

                if let Ok(res) = res {
                    return Ok(res);
                }
            }
            ColourSpecificationMethod::Enumerated(colourspace) => {
                if matches!(*colourspace, EnumeratedColourspace::RommRgb) {
                    return from_icc(
                        ROMM_PROFILE,
                        num_channels as u8,
                        has_alpha,
                        width,
                        height,
                        &interleaved,
                    );
                }
            }
            _ => {}
        }
    }

    let image = match (num_channels, has_alpha) {
        (1, false) => DynamicImage::ImageLuma8(
            ImageBuffer::from_raw(width, height, interleaved)
                .ok_or_else(|| "failed to build grayscale buffer".to_string())?,
        ),
        (2, true) => DynamicImage::ImageLumaA8(
            ImageBuffer::from_raw(width, height, interleaved)
                .ok_or_else(|| "failed to build grayscale-alpha buffer".to_string())?,
        ),
        (3, false) => DynamicImage::ImageRgb8(
            ImageBuffer::from_raw(width, height, interleaved)
                .ok_or_else(|| "failed to build rgb buffer".to_string())?,
        ),
        (4, true) => DynamicImage::ImageRgba8(
            ImageBuffer::from_raw(width, height, interleaved)
                .ok_or_else(|| "failed to build rgba buffer".to_string())?,
        ),
        (4, false) => from_icc(
            include_bytes!("../assets/CGATS001Compat-v2-micro.icc"),
            num_channels as u8,
            has_alpha,
            width,
            height,
            &interleaved,
        )?,
        _ => return Err("unsupported channel configuration".to_string()),
    };

    Ok(image)
}

static LOGGER: SimpleLogger = SimpleLogger;

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::LevelFilter::Warn
    }

    fn log(&self, record: &log::Record) {
        eprintln!("{}", record.args());
    }

    fn flush(&self) {}
}
