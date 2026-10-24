use crate::WORKSPACE_PATH;
use crate::harness::args::args;
use crate::harness::runner::Outcome;
use crate::harness::ui::count;
use image::{Rgba, RgbaImage, load_from_memory};
use std::cmp::max;
use std::fmt::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Suite {
    Raster,
    Svg,
    Write,
}

impl Suite {
    fn dir(self) -> &'static str {
        match self {
            Self::Raster => "raster",
            Self::Svg => "svg",
            Self::Write => "write",
        }
    }
}

/// Compares the rendered pages against the reference images of the snapshot `name`.
pub(crate) fn check(suite: Suite, name: &str, pages: Vec<Vec<u8>>) -> Outcome {
    if pages.is_empty() {
        return Outcome::Failed("document has no pages".into());
    }

    let (kind, stem) = ["pdfjs", "pdfbox", "pdfium", "corpus"]
        .into_iter()
        .find_map(|kind| Some((kind, name.strip_prefix(kind)?.strip_prefix('_')?)))
        .unwrap_or(("custom", name));

    let relative = Path::new(suite.dir()).join(kind);
    let refs_dir = WORKSPACE_PATH.join("snapshots").join(&relative);
    let diffs_dir = WORKSPACE_PATH.join("diffs").join(&relative);

    let mut created = 0;
    let mut updated = 0;
    let mut failures = vec![];

    for (index, page) in pages.iter().enumerate() {
        let file = if pages.len() == 1 {
            format!("{stem}.png")
        } else {
            format!("{stem}_p{index}.png")
        };
        let ref_path = refs_dir.join(&file);
        let diff_path = diffs_dir.join(&file);

        let Ok(reference) = std::fs::read(&ref_path) else {
            write_file(&ref_path, page);
            created += 1;
            continue;
        };

        let reference = load_from_memory(&reference).unwrap().into_rgba8();
        let actual = load_from_memory(page).unwrap().into_rgba8();
        let (diff_image, pixel_diff) = get_diff(&reference, &actual);

        if pixel_diff == 0 {
            let _ = std::fs::remove_file(&diff_path);
            continue;
        }

        std::fs::create_dir_all(&diffs_dir).unwrap();
        diff_image
            .save_with_format(&diff_path, image::ImageFormat::Png)
            .unwrap();

        if args().update {
            write_file(&ref_path, page);
            updated += 1;
        } else {
            let diff_path = diff_path
                .strip_prefix(&*WORKSPACE_PATH)
                .unwrap_or(&diff_path);
            failures.push((index, pixel_diff, diff_path.display().to_string()));
        }
    }

    match failures.as_slice() {
        [] if updated > 0 => Outcome::Updated(updated),
        [] if created > 0 => Outcome::Created(created),
        [] => Outcome::Passed,
        [(index, pixel_diff, path)] => {
            let page = if pages.len() == 1 {
                String::new()
            } else {
                format!(" on page {index}")
            };
            Outcome::Failed(format!(
                "{} pixels differ{page} → {path}",
                count(*pixel_diff as usize)
            ))
        }
        failures => {
            let mut message = format!("{} pages differ", failures.len());
            for (index, pixel_diff, path) in failures {
                write!(
                    message,
                    "\npage {index}: {} pixels differ → {path}",
                    count(*pixel_diff as usize)
                )
                .unwrap();
            }
            Outcome::Failed(message)
        }
    }
}

fn write_file(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

fn get_diff(expected_image: &RgbaImage, actual_image: &RgbaImage) -> (RgbaImage, u32) {
    let width = max(expected_image.width(), actual_image.width());
    let height = max(expected_image.height(), actual_image.height());

    let mut diff_image = RgbaImage::new(width * 3, height);

    let mut pixel_diff = 0;

    for x in 0..width {
        for y in 0..height {
            let actual_pixel = actual_image.get_pixel_checked(x, y);
            let expected_pixel = expected_image.get_pixel_checked(x, y);

            match (actual_pixel, expected_pixel) {
                (Some(actual), Some(expected)) => {
                    diff_image.put_pixel(x, y, *expected);
                    diff_image.put_pixel(x + 2 * width, y, *actual);
                    if is_pix_diff(expected, actual) {
                        pixel_diff += 1;
                        diff_image.put_pixel(x + width, y, diff_pixel(expected, actual));
                    } else {
                        diff_image.put_pixel(x + width, y, diff_pixel(expected, actual))
                    }
                }
                (Some(actual), None) => {
                    pixel_diff += 1;
                    diff_image.put_pixel(x + 2 * width, y, *actual);
                    diff_image.put_pixel(x + width, y, Rgba([255, 0, 0, 255]));
                }
                (None, Some(expected)) => {
                    pixel_diff += 1;
                    diff_image.put_pixel(x, y, *expected);
                    diff_image.put_pixel(x + width, y, Rgba([255, 0, 0, 255]));
                }
                _ => {
                    pixel_diff += 1;
                    diff_image.put_pixel(x, y, Rgba([255, 0, 0, 255]));
                    diff_image.put_pixel(x + width, y, Rgba([255, 0, 0, 255]));
                }
            }
        }
    }

    (diff_image, pixel_diff)
}

fn diff_pixel(pixel1: &Rgba<u8>, pixel2: &Rgba<u8>) -> Rgba<u8> {
    if pixel1.0[3] == 0 && pixel2.0[3] == 0 {
        return Rgba([0, 0, 0, 255]);
    }

    let max_channel_deviation = pixel1
        .0
        .iter()
        .zip(pixel2.0.iter())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);

    Rgba([max_channel_deviation, 0, 0, 255])
}

fn is_pix_diff(pixel1: &Rgba<u8>, pixel2: &Rgba<u8>) -> bool {
    if pixel1.0[3] == 0 && pixel2.0[3] == 0 {
        return false;
    }

    pixel1.0[0] != pixel2.0[0]
        || pixel1.0[1] != pixel2.0[1]
        || pixel1.0[2] != pixel2.0[2]
        || pixel1.0[3] != pixel2.0[3]
}
