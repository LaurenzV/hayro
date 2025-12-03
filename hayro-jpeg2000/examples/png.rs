use hayro_jpeg2000::{Bitmap, ColorSpace, DecodeSettings, read};
use image::{DynamicImage, ImageBuffer};
use moxcms::{ColorProfile, Layout, TransformOptions};
use std::env;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
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

    fn convert(bitmap: Bitmap, cs: ColorSpace) -> Result<DynamicImage, String> {
        let (width, height) = (bitmap.width, bitmap.height);
        let has_alpha = bitmap.has_alpha;

        let image = match (cs, has_alpha) {
            (hayro_jpeg2000::ColorSpace::Gray, false) => DynamicImage::ImageLuma8(
                ImageBuffer::from_raw(width, height, bitmap.data)
                    .ok_or_else(|| "failed to build grayscale buffer".to_string())?,
            ),
            (hayro_jpeg2000::ColorSpace::Gray, true) => DynamicImage::ImageLumaA8(
                ImageBuffer::from_raw(width, height, bitmap.data)
                    .ok_or_else(|| "failed to build grayscale-alpha buffer".to_string())?,
            ),
            (hayro_jpeg2000::ColorSpace::RGB, false) => DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(width, height, bitmap.data)
                    .ok_or_else(|| "failed to build rgb buffer".to_string())?,
            ),
            (hayro_jpeg2000::ColorSpace::RGB, true) => DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(width, height, bitmap.data)
                    .ok_or_else(|| "failed to build rgba buffer".to_string())?,
            ),
            (hayro_jpeg2000::ColorSpace::CMYK, false) => {
                from_icc(CMYK_PROFILE, 4, has_alpha, width, height, &bitmap.data)?
            }
            (hayro_jpeg2000::ColorSpace::CMYK, true) => {
                // moxcms doesn't support CMYK interleaved with alpha, so we
                // need to split it.
                let mut cmyk = vec![];
                let mut alpha = vec![];

                for sample in bitmap.data.chunks_exact(5) {
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
                hayro_jpeg2000::ColorSpace::Icc {
                    profile,
                    mut num_components,
                },
                has_alpha,
            ) => {
                if has_alpha {
                    num_components += 1;
                }

                from_icc(
                    &profile,
                    num_components,
                    has_alpha,
                    width,
                    height,
                    &bitmap.data,
                )
                .or_else(|e| match num_components {
                    1 => convert(bitmap, ColorSpace::Gray),
                    3 => convert(bitmap, ColorSpace::RGB),
                    4 => convert(bitmap, ColorSpace::CMYK),
                    _ => Err(e),
                })?
            }
        };

        Ok(image)
    }

    let cs = bitmap.color_space.clone();

    convert(bitmap, cs)
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
