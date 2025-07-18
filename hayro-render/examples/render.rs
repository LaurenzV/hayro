use hayro_interpret::font::FontQuery;
use hayro_interpret::font::standard_font::StandardFont;
use hayro_interpret::{FontData, InterpreterSettings};
use hayro_render::render_png;
use hayro_syntax::pdf::Pdf;
use std::sync::Arc;

fn main() {
    if let Ok(()) = log::set_logger(&LOGGER) {
        log::set_max_level(log::LevelFilter::Trace);
    }

    let file = std::fs::read("/Users/lstampfl/Programming/GitHub/sitro/pdf/in.pdf").unwrap();
    let data = Arc::new(file);
    let pdf = Pdf::new(data).unwrap();

    let settings = InterpreterSettings {
        font_resolver: Arc::new(|query| match query {
            FontQuery::Standard(s) => Some(get_standard(&s)),
            FontQuery::Fallback(f) => Some(get_standard(&f.pick_standard_font())),
        }),
        ..Default::default()
    };

    let pixmaps = render_png(&pdf, 1.0, settings, None).unwrap();

    std::fs::write("out.png", &pixmaps[0]).unwrap();
}

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

/// A simple stderr logger.
static LOGGER: SimpleLogger = SimpleLogger;
struct SimpleLogger;
impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::LevelFilter::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let target = if !record.target().is_empty() {
                record.target()
            } else {
                record.module_path().unwrap_or_default()
            };

            let line = record.line().unwrap_or(0);
            let args = record.args();

            match record.level() {
                log::Level::Error => eprintln!("Error (in {}:{}): {}", target, line, args),
                log::Level::Warn => eprintln!("Warning (in {}:{}): {}", target, line, args),
                log::Level::Info => eprintln!("Info (in {}:{}): {}", target, line, args),
                log::Level::Debug => eprintln!("Debug (in {}:{}): {}", target, line, args),
                log::Level::Trace => eprintln!("Trace (in {}:{}): {}", target, line, args),
            }
        }
    }

    fn flush(&self) {}
}
