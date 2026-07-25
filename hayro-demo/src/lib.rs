use console_error_panic_hook;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{RenderCache, RenderSettings};
use js_sys;
use vello_cpu::color::palette::css::WHITE;
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
            total_pages: 0,
        }
    }

    #[wasm_bindgen]
    pub fn load_pdf(&mut self, data: &[u8]) -> Result<(), JsValue> {
        let pdf = Pdf::new(data.to_vec()).map_err(|_| JsValue::from_str("Failed to parse PDF"))?;

        let pages = pdf.pages();

        self.total_pages = pages.len();
        self.pdf = Some(pdf);

        Ok(())
    }

    #[wasm_bindgen]
    pub fn render_page(
        &self,
        page_number: usize,
        viewport_width: f32,
        viewport_height: f32,
        device_pixel_ratio: f32,
        zoom: f32,
        fit_width: bool,
    ) -> Result<js_sys::Array, JsValue> {
        let pdf = self.pdf.as_ref().ok_or("No PDF loaded")?;
        let page = pdf
            .pages()
            .get(page_number.checked_sub(1).ok_or("Page out of bounds")?)
            .ok_or("Page out of bounds")?;

        let interpreter_settings = InterpreterSettings::default();
        let (base_width, base_height) = page.render_dimensions();

        // Calculate scale to fit in the viewport while accounting for the device pixel ratio.
        let target_width = viewport_width.max(1.0) * device_pixel_ratio.max(1.0);
        let target_height = viewport_height.max(1.0) * device_pixel_ratio.max(1.0);

        let scale_x = target_width / base_width;
        let scale_y = target_height / base_height;
        let fit_scale = if fit_width {
            scale_x
        } else {
            scale_x.min(scale_y)
        };
        let scale = fit_scale * zoom.clamp(0.1, 4.0);

        let render_settings = RenderSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
            ..Default::default()
        };

        let cache = RenderCache::new();
        let pixmap = hayro::render(page, &cache, &interpreter_settings, &render_settings);

        // Return array: [width, height, pixel_data]
        let result = js_sys::Array::new_with_length(3);
        result.set(0, JsValue::from(pixmap.width()));
        result.set(1, JsValue::from(pixmap.height()));

        // Cast Vec<Rgba8> to Vec<u8>
        let rgba_data = pixmap.take_unpremultiplied();
        let byte_data: Vec<u8> = bytemuck::cast_vec(rgba_data);
        result.set(2, JsValue::from(byte_data));

        Ok(result)
    }

    #[wasm_bindgen]
    pub fn get_page_dimensions(&self) -> Result<js_sys::Array, JsValue> {
        let pdf = self.pdf.as_ref().ok_or("No PDF loaded")?;
        let dimensions = js_sys::Array::new();

        for page in pdf.pages().iter() {
            let (width, height) = page.render_dimensions();
            dimensions.push(&JsValue::from(width));
            dimensions.push(&JsValue::from(height));
        }

        Ok(dimensions)
    }

    #[wasm_bindgen]
    pub fn get_total_pages(&self) -> usize {
        self.total_pages
    }
}
