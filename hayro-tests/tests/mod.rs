use hayro::FontQuery;
use hayro::Pdf;
use hayro::StandardFont;
use hayro::{FontData, InterpreterSettings};
use image::{Rgba, RgbaImage, load_from_memory};
use once_cell::sync::Lazy;
use resvg::tiny_skia::{Color, Pixmap};
use resvg::usvg::{Options, Transform, Tree};
use sitro::{RenderOptions, Renderer};
use std::cmp::max;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::Arc;

#[rustfmt::skip]
#[allow(non_snake_case)]
mod render;
mod fuzz;
mod svg;
mod write;

const REPLACE: Option<&str> = option_env!("REPLACE");
const STORE: Option<&str> = option_env!("STORE");

pub(crate) static WORKSPACE_PATH: Lazy<PathBuf> =
    Lazy::new(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(""));

pub(crate) static DIFFS_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let path = WORKSPACE_PATH.join("diffs");
    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::create_dir_all(&path);

    path
});
pub(crate) static RENDER_SNAPSHOTS_PATH: Lazy<PathBuf> =
    Lazy::new(|| WORKSPACE_PATH.join("snapshots/render"));
pub(crate) static SVG_SNAPSHOTS_PATH: Lazy<PathBuf> =
    Lazy::new(|| WORKSPACE_PATH.join("snapshots/svg"));
pub(crate) static WRITE_SNAPSHOTS_PATH: Lazy<PathBuf> =
    Lazy::new(|| WORKSPACE_PATH.join("snapshots/write"));
pub(crate) static STORE_PATH: Lazy<PathBuf> = Lazy::new(|| WORKSPACE_PATH.join("store"));

type RenderedDocument = Vec<Vec<u8>>;
type RenderedPage = Vec<u8>;

pub fn check_render(name: &str, snapshot_path: PathBuf, document: RenderedDocument) {
    let refs_path = if name.starts_with("pdfjs_") {
        snapshot_path.join("pdfjs")
    } else if name.starts_with("pdfbox_") {
        snapshot_path.join("pdfbox")
    } else {
        snapshot_path.clone()
    };

    // Ensure the snapshots subdirectory exists
    let _ = std::fs::create_dir_all(&refs_path);

    let snapshot_name = if let Some(name) = name.strip_prefix("pdfjs_") {
        name
    } else if let Some(name) = name.strip_prefix("pdfbox_") {
        name
    } else {
        name
    };

    let mut ref_created = false;
    let mut test_replaced = false;
    let mut failed = false;

    let mut check_single =
        |name: String, page: &RenderedPage, page_num: usize, failed: &mut bool| {
            let suffix = if document.len() == 1 {
                format!("{name}.png")
            } else {
                format!("{name}_{page_num}.png")
            };

            let ref_path = refs_path.join(&suffix);

            if !ref_path.exists() {
                std::fs::write(&ref_path, page).unwrap();
                oxipng::optimize(
                    &oxipng::InFile::Path(ref_path.clone()),
                    &oxipng::OutFile::from_path(ref_path),
                    &oxipng::Options::max_compression(),
                )
                .unwrap();
                ref_created = true;

                return;
            }

            let reference = load_from_memory(&std::fs::read(&ref_path).unwrap())
                .unwrap()
                .into_rgba8();
            let actual = load_from_memory(&document[page_num]).unwrap().into_rgba8();

            let (diff_image, pixel_diff) = get_diff(&reference, &actual);

            if pixel_diff > 0 {
                *failed = true;

                let diff_path = DIFFS_PATH.join(&suffix);
                diff_image
                    .save_with_format(&diff_path, ::image::ImageFormat::Png)
                    .unwrap();

                if REPLACE.is_some() {
                    std::fs::write(&ref_path, page).unwrap();
                    oxipng::optimize(
                        &oxipng::InFile::Path(ref_path.clone()),
                        &oxipng::OutFile::from_path(ref_path),
                        &oxipng::Options::max_compression(),
                    )
                    .unwrap();
                    test_replaced = true;
                }

                eprintln!("pixel diff was {pixel_diff}");
            }
        };

    if document.is_empty() {
        panic!("empty document");
    } else {
        for (index, page) in document.iter().enumerate() {
            check_single(snapshot_name.to_string(), page, index, &mut failed);
        }

        if test_replaced {
            panic!("test was replaced");
        } else if failed {
            panic!("at least one page had a pixel diff");
        }

        if ref_created {
            panic!("new reference image was created");
        }
    }
}

fn parse_range(range_str: &str) -> Option<RangeInclusive<usize>> {
    if range_str.contains("..=") {
        // Handle "3..=7" or "..=7"
        let parts: Vec<&str> = range_str.split("..=").collect();
        if parts.len() == 2 {
            if parts[0].is_empty() {
                // "..=7" - from start to 7
                if let Ok(end) = parts[1].parse::<usize>() {
                    return Some(0..=end);
                }
            } else {
                // "3..=7" - from 3 to 7
                if let (Ok(start), Ok(end)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>())
                {
                    return Some(start..=end);
                }
            }
        }
    } else if let Some(start_str) = range_str.strip_suffix("..") {
        // Handle "3.." - from 3 to end
        if let Ok(start) = start_str.parse::<usize>() {
            return Some(start..=usize::MAX);
        }
    }
    None
}

fn load_pdf(path: &str) -> Pdf {
    let path = WORKSPACE_PATH.join(path);
    let content = std::fs::read(&path).unwrap();
    let data = Arc::new(content);
    Pdf::new(data).unwrap()
}

fn interpreter_settings() -> InterpreterSettings {
    InterpreterSettings {
        font_resolver: Arc::new(|query| match query {
            FontQuery::Standard(s) => Some((get_standard(s), 0)),
            FontQuery::Fallback(f) => Some((get_standard(&f.pick_standard_font()), 0)),
        }),
        ..Default::default()
    }
}

pub fn run_render_test(name: &str, file_path: &str, range_str: Option<&str>) {
    let pdf = load_pdf(file_path);

    let settings = interpreter_settings();

    let range = range_str.and_then(parse_range);
    check_render(
        name,
        RENDER_SNAPSHOTS_PATH.clone(),
        hayro::render_pdf(&pdf, 1.0, settings, range)
            .unwrap()
            .into_iter()
            .map(|d| d.take_png())
            .collect(),
    );
}

pub fn run_svg_test(name: &str, file_path: &str, range_str: Option<&str>) {
    let pdf = load_pdf(file_path);

    let settings = interpreter_settings();

    let range = range_str.and_then(parse_range);

    let converted = pdf
        .pages()
        .iter()
        .enumerate()
        .flat_map(|(idx, p)| {
            if range.clone().is_some_and(|range| !range.contains(&idx)) {
                return None;
            }

            let svg = hayro_svg::convert(p, &settings);

            if STORE.is_some() {
                let _ = std::fs::create_dir_all(STORE_PATH.clone());

                std::fs::write(STORE_PATH.join(format!("{name}_{idx}.svg")), svg.as_bytes())
                    .unwrap();
            }

            let tree = Tree::from_data(svg.as_bytes(), &Options::default()).unwrap();
            let mut pixmap = Pixmap::new(
                tree.size().width().ceil() as u32,
                tree.size().height().ceil() as u32,
            )
            .unwrap();
            pixmap.fill(Color::WHITE);
            resvg::render(&tree, Transform::default(), &mut pixmap.as_mut());
            Some(pixmap.encode_png().unwrap())
        })
        .collect::<Vec<_>>();

    check_render(name, SVG_SNAPSHOTS_PATH.clone(), converted);
}

pub fn run_write_test(
    name: &str,
    file_path: &str,
    page_indices: &[usize],
    renderer: Renderer,
    page: bool,
) {
    let hayro_pdf = load_pdf(file_path);

    let buf = if page {
        hayro_write::extract_pages_to_pdf(&hayro_pdf, page_indices)
    } else {
        hayro_write::extract_pages_as_xobject_to_pdf(&hayro_pdf, page_indices)
    };

    if STORE.is_some() {
        let _ = std::fs::create_dir_all(STORE_PATH.clone());

        std::fs::write(STORE_PATH.join(format!("{name}.pdf")), &buf).unwrap();
    }

    let rendered = renderer
        .render_as_png(&buf, &RenderOptions::default())
        .unwrap();
    check_render(name, WRITE_SNAPSHOTS_PATH.clone(), rendered);
}

pub fn get_diff(expected_image: &RgbaImage, actual_image: &RgbaImage) -> (RgbaImage, u32) {
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
                        diff_image.put_pixel(x + width, y, Rgba([255, 0, 0, 255]));
                    } else {
                        diff_image.put_pixel(x + width, y, Rgba([0, 0, 0, 255]))
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

fn is_pix_diff(pixel1: &Rgba<u8>, pixel2: &Rgba<u8>) -> bool {
    if pixel1.0[3] == 0 && pixel2.0[3] == 0 {
        return false;
    }

    pixel1.0[0] != pixel2.0[0]
        || pixel1.0[1] != pixel2.0[1]
        || pixel1.0[2] != pixel2.0[2]
        || pixel1.0[3] != pixel2.0[3]
}

// We don't use the `embed-fonts` feature because we use the more complete liberation fonts for
// testing.
fn get_standard(font: &StandardFont) -> FontData {
    let data = match font {
        StandardFont::Helvetica => {
            &include_bytes!("../../assets/standard_fonts/LiberationSans-Regular.ttf")[..]
        }
        StandardFont::HelveticaBold => {
            &include_bytes!("../../assets/standard_fonts/LiberationSans-Bold.ttf")[..]
        }
        StandardFont::HelveticaOblique => {
            &include_bytes!("../../assets/standard_fonts/LiberationSans-Italic.ttf")[..]
        }
        StandardFont::HelveticaBoldOblique => {
            &include_bytes!("../../assets/standard_fonts/LiberationSans-BoldItalic.ttf")[..]
        }
        StandardFont::Courier => {
            &include_bytes!("../../assets/standard_fonts/LiberationMono-Regular.ttf")[..]
        }
        StandardFont::CourierBold => {
            &include_bytes!("../../assets/standard_fonts/LiberationMono-Bold.ttf")[..]
        }
        StandardFont::CourierOblique => {
            &include_bytes!("../../assets/standard_fonts/LiberationMono-Italic.ttf")[..]
        }
        StandardFont::CourierBoldOblique => {
            &include_bytes!("../../assets/standard_fonts/LiberationMono-BoldItalic.ttf")[..]
        }
        StandardFont::TimesRoman => {
            &include_bytes!("../../assets/standard_fonts/LiberationSerif-Regular.ttf")[..]
        }
        StandardFont::TimesBold => {
            &include_bytes!("../../assets/standard_fonts/LiberationSerif-Bold.ttf")[..]
        }
        StandardFont::TimesItalic => {
            &include_bytes!("../../assets/standard_fonts/LiberationSerif-Italic.ttf")[..]
        }
        StandardFont::TimesBoldItalic => {
            &include_bytes!("../../assets/standard_fonts/LiberationSerif-BoldItalic.ttf")[..]
        }
        StandardFont::ZapfDingBats => {
            &include_bytes!("../../assets/standard_fonts/FoxitDingbats.pfb")[..]
        }
        StandardFont::Symbol => &include_bytes!("../../assets/standard_fonts/FoxitSymbol.pfb")[..],
    };

    Arc::new(data)
}
