use console_error_panic_hook;
use hayro::{FontQuery, InterpreterSettings, Pdf, RenderSettings};
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
        let page = pdf
            .pages()
            .get(self.current_page)
            .ok_or("Page out of bounds")?;

        if self.current_page >= self.total_pages {
            return Err(JsValue::from_str("Page out of bounds"));
        }

        // TODO: Fetch fonts lazily
        let interpreter_settings = InterpreterSettings {
            font_resolver: Arc::new(|query| match query {
                FontQuery::Standard(s) => Some(s.get_font_data()),
                FontQuery::Fallback(f) => Some(f.pick_standard_font().get_font_data()),
            }),
            ..Default::default()
        };

        let render_settings = RenderSettings {
            x_scale: 2.0,
            y_scale: 2.0,
            ..Default::default()
        };

        let pixmap = hayro::render(page, &interpreter_settings, &render_settings);

        Ok(pixmap.into_png().unwrap())
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
