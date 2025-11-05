use hayro_jpeg2000::read;
use image::{DynamicImage, ImageBuffer};
use moxcms::{ColorProfile, Layout, TransformOptions};
use std::env;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

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

    let bitmap = read(&data).map_err(|err| format!("decode error: {err}"))?;

    let (width, height) = (bitmap.metadata.width, bitmap.metadata.height);
    let has_alpha = bitmap.channels.iter().any(|c| c.is_alpha);
    let num_channels = bitmap.channels.len();

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

    let dynamic = match (num_channels, has_alpha) {
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
        (4, false) => {
            let src_profile = ColorProfile::new_from_slice(include_bytes!(
                "../assets/CGATS001Compat-v2-micro.icc"
            ))
            .unwrap();
            let dest_profile = ColorProfile::new_srgb();

            let src_layout = Layout::Rgba;
            let transform = src_profile
                .create_transform_8bit(
                    src_layout,
                    &dest_profile,
                    Layout::Rgb,
                    TransformOptions::default(),
                )
                .unwrap();

            let mut dest = vec![0; (width * height * 3) as usize];

            transform.transform(&interleaved, &mut dest).unwrap();

            DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(width, height, dest)
                    .ok_or_else(|| "failed to build rgb buffer".to_string())?,
            )
        }
        _ => return Err("unsupported channel configuration".to_string()),
    };

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
