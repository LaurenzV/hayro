//! This example shows you how you can render a PDF file to PNG.

use fontdb::{Family, Query, Weight};
use hayro::hayro_interpret::font::{FontData, FontQuery};
use hayro::hayro_interpret::hayro_cmap::CidFamily;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{RenderSettings, render};
use std::sync::Arc;
use vello_cpu::color::palette::css::WHITE;

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

    let mut fontdb = fontdb::Database::new();
    fontdb.load_system_fonts();

    let interpreter_settings = InterpreterSettings {
        font_resolver: Arc::new(move |query| match query {
            FontQuery::Standard(s) => Some(s.get_font_data()),
            FontQuery::Fallback(f) => {
                if f.character_collection
                    .as_ref()
                    .is_none_or(|c| matches!(c.family, CidFamily::AdobeIdentity))
                {
                    return Some(f.pick_standard_font().get_font_data());
                }

                let query = Query {
                    families: &[Family::Name("Noto Sans SC")],
                    weight: if f.is_bold {
                        Weight::BOLD
                    } else {
                        Weight::NORMAL
                    },
                    ..Default::default()
                };

                let id = fontdb.query(&query)?;
                let mut font_data: Option<(FontData, u32)> = None;

                fontdb.with_face_data(id, |data, idx| {
                    font_data = Some((Arc::new(data.to_vec()), idx));
                });

                font_data
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
