use crate::harness::args::Args;
use crate::harness::runner::Test;
use crate::harness::snapshot::encode_png;
use crate::harness::ui::Ui;
use crate::harness::{args, assets, references, report, runner, ui};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::font::{FontData, FontQuery, StandardFont};
use hayro::hayro_interpret::hayro_cmap::CidFamily;
use hayro::vello_cpu::Pixels;
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro_svg::SvgRenderSettings;
use hayro_syntax::Pdf;
use hayro_syntax::{DecryptionError, LoadPdfError};
use image::buffer::ConvertBuffer;
use image::{RgbImage, RgbaImage};
use resvg::tiny_skia::{Color, Pixmap, PixmapPaint};
use resvg::usvg::{Options, Transform, Tree};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

mod harness;
mod load;
mod raster;
mod svg;
mod text;
mod write;

pub(crate) static WORKSPACE_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));

fn main() {
    if std::env::args().any(|arg| arg == "--bench") {
        return;
    }

    let args = args::init();
    let (tests, ignored) = collect(args);

    if args.list {
        for test in &tests {
            println!("{}", test.name);
        }
        eprintln!("{} tests", ui::count(tests.len()));
        return;
    }

    if tests.is_empty() {
        eprintln!("no tests match the given filters");
        return;
    }

    report::clear();
    if let Some(dir) = &args.ci {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
    }

    let jobs = args
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));

    let mut suites = BTreeMap::new();
    for test in &tests {
        *suites.entry(test.suite().to_string()).or_default() += 1;
    }

    let mut ui = Ui::new();
    ui.header(&suites, tests.len(), jobs.min(tests.len()));

    let mut unavailable = assets::fetch(&tests, args.offline, &mut ui);
    let (blocked, runnable): (Vec<_>, Vec<_>) = tests
        .into_iter()
        .partition(|test| unavailable.contains_key(&test.name));

    ui.tests_started(runnable.len() + blocked.len());
    let mut not_run = vec![];
    for test in blocked {
        let outcome = unavailable.remove(&test.name).unwrap();
        let result = runner::TestResult {
            name: test.name,
            outcome,
            duration: Default::default(),
        };
        ui.finished(&result);
        not_run.push(result);
    }

    let mut results = runner::run(runnable, jobs, &mut ui);
    results.append(&mut not_run);
    if let Some(dir) = &args.ci {
        references::write_lockfile(dir);
    }
    let report = report::write(&results);
    ui.summary(&results, ignored, report.as_deref());

    let failed = results
        .iter()
        .any(|result| matches!(result.outcome, runner::Outcome::Failed(_)));
    if failed {
        std::process::exit(1);
    }
}

fn collect(args: &Args) -> (Vec<Test>, usize) {
    let (raster, ignored) = raster::tests();

    let mut tests: Vec<Test> = raster
        .into_iter()
        .chain(svg::tests())
        .chain(write::tests())
        .chain(load::tests())
        .chain(text::tests())
        .chain([
            Test::unit("visibility", visibility),
            Test::unit(
                "aliasing_threshold_applies_to_every_fill",
                aliasing_threshold_applies_to_every_fill,
            ),
        ])
        .filter(|test| args.matches(&test.name))
        .collect();
    tests.sort_by(|a, b| a.name.cmp(&b.name));

    let ignored = ignored.iter().filter(|name| args.matches(name)).count();

    (tests, ignored)
}

pub(crate) fn load_pdf(path: &str) -> Pdf {
    try_load_pdf(path, "").unwrap()
}

pub(crate) fn try_load_pdf(path: &str, password: &str) -> Result<Pdf, String> {
    let content = std::fs::read(WORKSPACE_PATH.join(path))
        .map_err(|err| format!("couldn't read {path}: {err}"))?;
    Pdf::new_with_password(content, password)
        .map_err(|err| format!("couldn't load {path}: {err:?}"))
}

pub(crate) fn interpreter_settings() -> InterpreterSettings {
    InterpreterSettings {
        font_resolver: Arc::new(|query| match query {
            FontQuery::Standard(s) => Some((get_standard(s)?, 0)),
            FontQuery::Fallback(f) => {
                if let Some(data) = get_noto_fallback(f) {
                    return Some((data, 0));
                }

                Some((get_standard(&f.pick_standard_font())?, 0))
            }
        }),
        ..Default::default()
    }
}

fn read_font(name: &str) -> Option<FontData> {
    let path = WORKSPACE_PATH.join("assets").join(name);
    Some(Arc::new(std::fs::read(&path).ok()?))
}

fn get_noto_fallback(query: &hayro::hayro_interpret::font::FallbackFontQuery) -> Option<FontData> {
    let family = &query.character_collection.as_ref()?.family;

    let name = match family {
        CidFamily::AdobeGB1 | CidFamily::AdobeCNS1 => {
            if query.is_bold {
                "NotoSansCJKsc-Bold.otf"
            } else {
                "NotoSansCJKsc-Regular.otf"
            }
        }
        CidFamily::AdobeJapan1 => {
            if query.is_bold {
                "NotoSansCJKjp-Bold.otf"
            } else {
                "NotoSansCJKjp-Regular.otf"
            }
        }
        CidFamily::AdobeKorea1 => {
            if query.is_bold {
                "NotoSansCJKkr-Bold.otf"
            } else {
                "NotoSansCJKkr-Regular.otf"
            }
        }
        _ => return None,
    };

    read_font(name)
}

pub(crate) fn svg_render_settings() -> SvgRenderSettings {
    SvgRenderSettings {
        bg_color: [0, 0, 0, 0],
    }
}

pub(crate) fn render_pdf(
    pdf: &Pdf,
    name: &str,
    settings: InterpreterSettings,
    range: Option<RangeInclusive<usize>>,
) -> Vec<RgbaImage> {
    hayro::render_pdf(pdf, 1.0, settings, range)
        .unwrap()
        .into_iter()
        .enumerate()
        .map(|(idx, pixmap)| {
            let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
            let page = match pixmap.try_take_rgb8(ImageAlphaType::Alpha) {
                Pixels::Rgb8(data) => RgbImage::from_raw(width, height, data).unwrap().convert(),
                Pixels::Rgba8(data) => RgbaImage::from_raw(width, height, data).unwrap(),
            };

            if args::args().store {
                let dir = WORKSPACE_PATH.join("store/pdf");
                let _ = std::fs::create_dir_all(&dir);

                std::fs::write(dir.join(format!("{name}_{idx}.png")), encode_png(&page)).unwrap();
            }

            page
        })
        .collect()
}

pub(crate) fn render_svg(
    pdf: &Pdf,
    name: &str,
    interpreter_settings: InterpreterSettings,
    render_settings: SvgRenderSettings,
    range: Option<RangeInclusive<usize>>,
) -> Vec<RgbaImage> {
    let cache = hayro_svg::RenderCache::new();
    pdf.pages()
        .iter()
        .enumerate()
        .flat_map(|(idx, p)| {
            if range.clone().is_some_and(|range| !range.contains(&idx)) {
                return None;
            }

            let svg = hayro_svg::convert(p, &cache, &interpreter_settings, &render_settings);

            if args::args().store {
                let dir = WORKSPACE_PATH.join("store/svg");
                let _ = std::fs::create_dir_all(&dir);

                std::fs::write(dir.join(format!("{name}_{idx}.svg")), svg.as_bytes()).unwrap();
            }

            let tree = Tree::from_data(svg.as_bytes(), &Options::default()).unwrap();
            let mut rendered = Pixmap::new(
                tree.size().width().ceil() as u32,
                tree.size().height().ceil() as u32,
            )
            .unwrap();
            resvg::render(&tree, Transform::default(), &mut rendered.as_mut());

            // A workaround for https://github.com/linebender/resvg/issues/1069
            let mut composited = Pixmap::new(rendered.width(), rendered.height()).unwrap();
            composited.fill(Color::from_rgba8(255, 255, 255, 255));
            composited.draw_pixmap(
                0,
                0,
                rendered.as_ref(),
                &PixmapPaint::default(),
                Transform::default(),
                None,
            );

            let data = composited
                .pixels()
                .iter()
                .flat_map(|pixel| {
                    let pixel = pixel.demultiply();
                    [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
                })
                .collect();

            Some(RgbaImage::from_raw(composited.width(), composited.height(), data).unwrap())
        })
        .collect::<Vec<_>>()
}

// We don't use the `embed-fonts` feature because we use the more complete liberation fonts for
// testing.
fn get_standard(font: &StandardFont) -> Option<FontData> {
    match font {
        StandardFont::Helvetica => Some(Arc::new(
            &include_bytes!("../assets/LiberationSans-Regular.ttf")[..],
        )),
        StandardFont::HelveticaBold => Some(Arc::new(
            &include_bytes!("../assets/LiberationSans-Bold.ttf")[..],
        )),
        StandardFont::HelveticaOblique => Some(Arc::new(
            &include_bytes!("../assets/LiberationSans-Italic.ttf")[..],
        )),
        StandardFont::HelveticaBoldOblique => Some(Arc::new(
            &include_bytes!("../assets/LiberationSans-BoldItalic.ttf")[..],
        )),
        StandardFont::Courier => Some(Arc::new(
            &include_bytes!("../assets/LiberationMono-Regular.ttf")[..],
        )),
        StandardFont::CourierBold => Some(Arc::new(
            &include_bytes!("../assets/LiberationMono-Bold.ttf")[..],
        )),
        StandardFont::CourierOblique => Some(Arc::new(
            &include_bytes!("../assets/LiberationMono-Italic.ttf")[..],
        )),
        StandardFont::CourierBoldOblique => Some(Arc::new(
            &include_bytes!("../assets/LiberationMono-BoldItalic.ttf")[..],
        )),
        StandardFont::TimesRoman => Some(Arc::new(
            &include_bytes!("../assets/LiberationSerif-Regular.ttf")[..],
        )),
        StandardFont::TimesBold => Some(Arc::new(
            &include_bytes!("../assets/LiberationSerif-Bold.ttf")[..],
        )),
        StandardFont::TimesItalic => Some(Arc::new(
            &include_bytes!("../assets/LiberationSerif-Italic.ttf")[..],
        )),
        StandardFont::TimesBoldItalic => Some(Arc::new(
            &include_bytes!("../assets/LiberationSerif-BoldItalic.ttf")[..],
        )),
        StandardFont::ZapfDingBats => Some(Arc::new(
            &include_bytes!("../../hayro-interpret/assets/FoxitDingbats.pfb")[..],
        )),
        StandardFont::Symbol => Some(Arc::new(
            &include_bytes!("../../hayro-interpret/assets/FoxitSymbol.pfb")[..],
        )),
    }
}

fn visibility() {
    #[expect(dead_code)]
    fn decryption(error: &LoadPdfError) {
        match error {
            LoadPdfError::Decryption(d) => match d {
                DecryptionError::MissingIDEntry => {}
                DecryptionError::PasswordProtected => {}
                DecryptionError::InvalidEncryption => {}
                DecryptionError::UnsupportedAlgorithm => {}
            },
            LoadPdfError::Invalid => {}
        }
    }
}

fn single_page_pdf(content: &str) -> Vec<u8> {
    let pattern = "0 1 0 rg 1 1 m 1 9 9 9 9 1 c f";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] \
            /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        format!(
            "<< /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 \
                /YStep 10 /Resources << >> /Length {} >>\nstream\n{pattern}\nendstream",
            pattern.len()
        ),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn aliasing_threshold_applies_to_every_fill() {
    // Exercise images, subsequent fills, and a tiling pattern's child context.
    let content = "0 0 1 rg 5 20 m 5 35 20 38 35 20 c f \
        q 20 10 -10 20 12 2 cm BI /W 2 /H 2 /CS /G /BPC 8 ID \x00\x00\x00\x00 EI Q \
        1 0 0 rg 2 2 m 30 8 l 10 16 l f \
        /Pattern cs /P0 scn 22 22 15 15 re f";
    let pdf = Pdf::new(single_page_pdf(content)).unwrap();
    let page = &pdf.pages()[0];
    let render = |aliasing_threshold| {
        let settings = hayro::RenderSettings {
            aliasing_threshold,
            ..Default::default()
        };
        hayro::render(
            page,
            &hayro::RenderCache::new(),
            &interpreter_settings(),
            &settings,
            &hayro::PixmapSettings {
                x_scale: 3.0,
                y_scale: 3.0,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            },
        )
        .take_rgba8(hayro::vello_cpu::peniko::ImageAlphaType::Alpha)
    };
    let partial = |data: &[u8]| {
        data.as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[..3].iter().any(|value| !matches!(value, 0 | 255)))
            .count()
    };

    assert!(partial(&render(None)) > 0);
    assert_eq!(partial(&render(Some(127))), 0);
}
