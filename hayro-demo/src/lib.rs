use console_error_panic_hook;
use hayro_render::{FontData, FontQuery, InterpreterSettings, StandardFont};
use hayro_syntax::Pdf;
use js_sys;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

struct ConsoleLogger;

impl log::Log for ConsoleLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::LevelFilter::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let message = format!(
                "[{}:{}] {}",
                record.target(),
                record.line().unwrap_or(0),
                record.args()
            );

            let level_str = match record.level() {
                log::Level::Error => "error",
                log::Level::Warn => "warn",
                log::Level::Info => "info",
                log::Level::Debug => "debug",
                log::Level::Trace => "trace",
            };

            match record.level() {
                log::Level::Error => web_sys::console::error_1(&message.clone().into()),
                log::Level::Warn => web_sys::console::warn_1(&message.clone().into()),
                _ => web_sys::console::log_1(&message.clone().into()),
            }

            if let Some(window) = web_sys::window() {
                if let Ok(add_log_entry) = js_sys::Reflect::get(&window, &"addLogEntry".into()) {
                    if add_log_entry.is_function() {
                        let function = js_sys::Function::from(add_log_entry);
                        let _ = function.call2(&window, &level_str.into(), &message.into());
                    }
                }
            }
        }
    }

    fn flush(&self) {}
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

static LOGGER: ConsoleLogger = ConsoleLogger;

#[wasm_bindgen]
pub struct PdfViewer {
    pdf: Option<Pdf>,
    current_page: usize,
    total_pages: usize,
}

#[wasm_bindgen]
impl PdfViewer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        console_error_panic_hook::set_once();

        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Warn);
        }

        Self {
            pdf: None,
            current_page: 0,
            total_pages: 0,
        }
    }

    #[wasm_bindgen]
    pub fn load_pdf(&mut self, data: &[u8]) -> Result<(), JsValue> {
        let pdf = Pdf::new(Arc::new(data.to_vec()))
            .map_err(|_| JsValue::from_str("Failed to parse PDF"))?;

        let pages = pdf.pages();

        self.total_pages = pages.len();
        self.pdf = Some(pdf);
        self.current_page = 0;

        Ok(())
    }

    #[wasm_bindgen]
    pub fn render_current_page(&self) -> Result<Vec<u8>, JsValue> {
        let pdf = self.pdf.as_ref().ok_or("No PDF loaded")?;

        if self.current_page >= self.total_pages {
            return Err(JsValue::from_str("Page out of bounds"));
        }

        // TODO: Fetch fonts lazily
        let settings = InterpreterSettings {
            font_resolver: Arc::new(|query| match query {
                FontQuery::Standard(s) => Some(get_standard(&s)),
                FontQuery::Fallback(f) => Some(get_standard(&f.pick_standard_font())),
            }),
            ..Default::default()
        };

        let pixmaps = hayro_render::render_png(
            pdf,
            2.0,
            settings,
            Some(self.current_page..=self.current_page),
        );

        pixmaps
            .as_ref()
            .and_then(|p| p.first())
            .cloned()
            .ok_or_else(|| JsValue::from_str("Failed to render page"))
    }

    #[wasm_bindgen]
    pub fn next_page(&mut self) -> bool {
        if self.current_page + 1 < self.total_pages {
            self.current_page += 1;
            true
        } else {
            false
        }
    }

    #[wasm_bindgen]
    pub fn previous_page(&mut self) -> bool {
        if self.current_page > 0 {
            self.current_page -= 1;
            true
        } else {
            false
        }
    }

    #[wasm_bindgen]
    pub fn set_page(&mut self, page: usize) -> bool {
        if page > 0 && page <= self.total_pages {
            self.current_page = page - 1;
            true
        } else {
            false
        }
    }

    #[wasm_bindgen]
    pub fn get_current_page(&self) -> usize {
        self.current_page + 1
    }

    #[wasm_bindgen]
    pub fn get_total_pages(&self) -> usize {
        self.total_pages
    }
}
