//! This example shows you how you can render a PDF file to PNG.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::font::{FontData, FontQuery, StandardFont};
use hayro::hayro_interpret::hayro_cmap::CidFamily;
use hayro::hayro_syntax::Pdf;
use hayro::{RenderSettings, render};
use std::path::Path;
use std::sync::Arc;
use vello_cpu::color::palette::css::WHITE;

fn load_asset(name: &str) -> Option<(FontData, u32)> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../hayro-tests/assets");
    let path = base.join(name);
    let data = std::fs::read(&path).ok()?;
    Some((Arc::new(data), 0))
}

fn main() {
    if let Ok(()) = log::set_logger(&LOGGER) {
        log::set_max_level(log::LevelFilter::Trace);
    }

    let file = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let output_dir = std::env::args().nth(2).unwrap_or_else(|| ".".to_string());
    let scale = std::env::args()
        .nth(3)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&output_dir).unwrap();

    let pdf = Pdf::new(file).unwrap();

    let interpreter_settings = InterpreterSettings {
        font_resolver: Arc::new(move |query| match query {
            FontQuery::Standard(s) => {
                let name = pick_standard_font(s);
                load_asset(name).or_else(|| Some(s.get_font_data()))
            }
            FontQuery::Fallback(f) => {
                if let Some(cc) = &f.character_collection {
                    let name = match cc.family {
                        CidFamily::AdobeGB1 | CidFamily::AdobeCNS1 => {
                            if f.is_bold {
                                "NotoSansCJKsc-Bold.otf"
                            } else {
                                "NotoSansCJKsc-Regular.otf"
                            }
                        }
                        CidFamily::AdobeJapan1 => {
                            if f.is_bold {
                                "NotoSansCJKjp-Bold.otf"
                            } else {
                                "NotoSansCJKjp-Regular.otf"
                            }
                        }
                        CidFamily::AdobeKorea1 => {
                            if f.is_bold {
                                "NotoSansCJKkr-Bold.otf"
                            } else {
                                "NotoSansCJKkr-Regular.otf"
                            }
                        }
                        _ => {
                            let name = pick_standard_font(&f.pick_standard_font());
                            return load_asset(name)
                                .or_else(|| Some(f.pick_standard_font().get_font_data()));
                        }
                    };

                    if let Some(data) = load_asset(name) {
                        return Some(data);
                    }
                }

                let name = pick_standard_font(&f.pick_standard_font());
                load_asset(name).or_else(|| Some(f.pick_standard_font().get_font_data()))
            }
        }),
        ..Default::default()
    };

    let render_settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        bg_color: WHITE,
        ..Default::default()
    };

    for (idx, page) in pdf.pages().iter().enumerate() {
        let pixmap = render(page, &interpreter_settings, &render_settings);
        let output_path = format!("{}/rendered_{idx}.png", output_dir);
        std::fs::write(output_path, pixmap.into_png().unwrap()).unwrap();
    }
}

fn pick_standard_font(font: &StandardFont) -> &'static str {
    match font {
        StandardFont::Helvetica => "LiberationSans-Regular.ttf",
        StandardFont::HelveticaBold => "LiberationSans-Bold.ttf",
        StandardFont::HelveticaOblique => "LiberationSans-Italic.ttf",
        StandardFont::HelveticaBoldOblique => "LiberationSans-BoldItalic.ttf",
        StandardFont::Courier => "LiberationMono-Regular.ttf",
        StandardFont::CourierBold => "LiberationMono-Bold.ttf",
        StandardFont::CourierOblique => "LiberationMono-Italic.ttf",
        StandardFont::CourierBoldOblique => "LiberationMono-BoldItalic.ttf",
        StandardFont::TimesRoman => "LiberationSerif-Regular.ttf",
        StandardFont::TimesBold => "LiberationSerif-Bold.ttf",
        StandardFont::TimesItalic => "LiberationSerif-Italic.ttf",
        StandardFont::TimesBoldItalic => "LiberationSerif-BoldItalic.ttf",
        StandardFont::ZapfDingBats => "FoxitDingbats.pfb",
        StandardFont::Symbol => "FoxitSymbol.pfb",
    }
}

/// A simple stderr logger.
static LOGGER: SimpleLogger = SimpleLogger;
struct SimpleLogger;
impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::LevelFilter::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let target = if !record.target().is_empty() {
                record.target()
            } else {
                record.module_path().unwrap_or_default()
            };

            let line = record.line().unwrap_or(0);
            let args = record.args();

            match record.level() {
                log::Level::Error => eprintln!("Error (in {target}:{line}): {args}"),
                log::Level::Warn => eprintln!("Warning (in {target}:{line}): {args}"),
                log::Level::Info => eprintln!("Info (in {target}:{line}): {args}"),
                log::Level::Debug => eprintln!("Debug (in {target}:{line}): {args}"),
                log::Level::Trace => eprintln!("Trace (in {target}:{line}): {args}"),
            }
        }
    }

    fn flush(&self) {}
}
