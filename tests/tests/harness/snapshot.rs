use crate::WORKSPACE_PATH;
use crate::harness::args::args;
use crate::harness::report;
use crate::harness::runner::Outcome;
use crate::harness::ui::count;
use image::{GenericImageView, Rgba, RgbaImage, load_from_memory};
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

/// Compares the rendered pages against the reference images of the snapshot `name`, allowing up
/// to `max_diff_pixels` differing pixels per page.
pub(crate) fn check(
    suite: Suite,
    name: &str,
    pages: Vec<Vec<u8>>,
    max_diff_pixels: u32,
) -> Outcome {
    if pages.is_empty() {
        return Outcome::Failed("document has no pages".into());
    }

    let (kind, stem) = ["pdfjs", "pdfbox", "pdfium", "corpus"]
        .into_iter()
        .find_map(|kind| Some((kind, name.strip_prefix(kind)?.strip_prefix('_')?)))
        .unwrap_or(("custom", name));

    let relative = Path::new(suite.dir()).join(kind);
    let refs_dir = WORKSPACE_PATH.join("snapshots").join(&relative);

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

        let Ok(reference) = std::fs::read(&ref_path) else {
            if args().update {
                write_file(&ref_path, page);
                created += 1;
            } else {
                failures.push((index, "missing reference image".to_string()));
                report::add(report::Page::Missing {
                    index: (pages.len() > 1).then_some(index),
                    size: load_from_memory(page).unwrap().dimensions(),
                    png: page.clone(),
                });
            }
            continue;
        };

        let reference = load_from_memory(&reference).unwrap().into_rgba8();
        let actual = load_from_memory(page).unwrap().into_rgba8();
        let diff = get_diff(&reference, &actual);

        if diff.pixels <= max_diff_pixels {
            continue;
        }

        if args().update {
            write_file(&ref_path, page);
            updated += 1;
            continue;
        }

        let report_dir = report::files_dir().join(&relative);
        let diff_file = file.replace(".png", ".diff.png");
        write_file(&report_dir.join(&file), page);
        diff.mask
            .save_with_format(report_dir.join(&diff_file), image::ImageFormat::Png)
            .unwrap();

        let relative = relative.to_string_lossy().replace('\\', "/");
        report::add(report::Page::Changed {
            index: (pages.len() > 1).then_some(index),
            reference: format!("snapshots/{relative}/{file}"),
            actual: format!("report/{relative}/{file}"),
            diff: format!("report/{relative}/{diff_file}"),
            pixels: diff.pixels,
            max_delta: diff.max_delta,
            reference_size: reference.dimensions(),
            actual_size: actual.dimensions(),
        });
        failures.push((
            index,
            format!(
                "{} pixels differ (max deviation {})",
                count(diff.pixels as usize),
                diff.max_delta
            ),
        ));
    }

    match failures.as_slice() {
        [] if updated > 0 => Outcome::Updated(updated),
        [] if created > 0 => Outcome::Created(created),
        [] => Outcome::Passed,
        [(index, failure)] if pages.len() > 1 => {
            Outcome::Failed(format!("page {index}: {failure}"))
        }
        [(_, failure)] => Outcome::Failed(failure.clone()),
        failures => {
            let mut message = format!("{} pages failed", failures.len());
            for (index, failure) in failures {
                write!(message, "\npage {index}: {failure}").unwrap();
            }
            Outcome::Failed(message)
        }
    }
}

fn write_file(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

struct Diff {
    pixels: u32,
    max_delta: u8,
    mask: RgbaImage,
}

fn get_diff(expected: &RgbaImage, actual: &RgbaImage) -> Diff {
    let width = max(expected.width(), actual.width());
    let height = max(expected.height(), actual.height());

    let mut mask = RgbaImage::new(width, height);
    let mut pixels = 0;
    let mut max_delta = 0;

    for y in 0..height {
        for x in 0..width {
            let delta = match (
                expected.get_pixel_checked(x, y),
                actual.get_pixel_checked(x, y),
            ) {
                (Some(expected), Some(actual)) => deviation(expected, actual),
                _ => 255,
            };

            if delta == 0 {
                continue;
            }

            pixels += 1;
            max_delta = max_delta.max(delta);
            let color = match delta {
                1..=2 => [201, 164, 0, 255],
                3..=20 => [224, 123, 0, 255],
                _ => [217, 48, 37, 255],
            };
            mask.put_pixel(x, y, Rgba(color));
        }
    }

    add_halo(&mut mask);

    Diff {
        pixels,
        max_delta,
        mask,
    }
}

// Single differing pixels disappear when the report shows a page scaled down, so they are
// surrounded by a translucent halo of the same color.
fn add_halo(mask: &mut RgbaImage) {
    const RADIUS: i64 = 3;

    let exact = mask.clone();
    let (width, height) = (i64::from(mask.width()), i64::from(mask.height()));
    for (x, y, pixel) in exact.enumerate_pixels() {
        if pixel.0[3] != 255 {
            continue;
        }
        for dy in -RADIUS..=RADIUS {
            for dx in -RADIUS..=RADIUS {
                let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                if nx < 0 || ny < 0 || nx >= width || ny >= height {
                    continue;
                }
                let target = mask.get_pixel_mut(nx as u32, ny as u32);
                if target.0[3] == 0 {
                    *target = Rgba([pixel.0[0], pixel.0[1], pixel.0[2], 90]);
                }
            }
        }
    }
}

fn deviation(pixel1: &Rgba<u8>, pixel2: &Rgba<u8>) -> u8 {
    if pixel1.0[3] == 0 && pixel2.0[3] == 0 {
        return 0;
    }

    pixel1
        .0
        .iter()
        .zip(pixel2.0.iter())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}
