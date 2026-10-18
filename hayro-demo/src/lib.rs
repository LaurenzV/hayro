use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use vello_cpu::color::palette::css::WHITE;
use vello_cpu::peniko::ImageAlphaType;
use wasm_bindgen::prelude::*;

struct ConsoleLogger;

impl log::Log for ConsoleLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::LevelFilter::Warn
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let message = format!(
            "[{}:{}] {}",
            record.target(),
            record.line().unwrap_or(0),
            record.args()
        );

        match record.level() {
            log::Level::Error => web_sys::console::error_1(&message.into()),
            log::Level::Warn => web_sys::console::warn_1(&message.into()),
            _ => web_sys::console::log_1(&message.into()),
        }
    }

    fn flush(&self) {}
}

static LOGGER: ConsoleLogger = ConsoleLogger;

/// A loaded PDF document that renders its pages on request.
#[wasm_bindgen]
pub struct PdfViewer {
    pdf: Pdf,
}

#[wasm_bindgen]
impl PdfViewer {
    /// Parses the PDF in `data`.
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8]) -> Result<Self, JsValue> {
        console_error_panic_hook::set_once();

        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Warn);
        }

        let pdf = Pdf::new(data.to_vec()).map_err(|_| JsValue::from_str("Failed to parse PDF"))?;

        Ok(Self { pdf })
    }

    /// The number of pages.
    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// The width and height of every page in PDF points, interleaved.
    pub fn page_sizes(&self) -> Vec<f32> {
        self.pdf
            .pages()
            .iter()
            .flat_map(|page| {
                let (width, height) = page.render_dimensions();
                [width, height]
            })
            .collect()
    }

    /// Renders the page at `index` with `scale` pixels per PDF point.
    ///
    /// Returns an array of the pixmap's width, height and RGBA pixels.
    pub fn render_page(&self, index: usize, scale: f32) -> Result<js_sys::Array, JsValue> {
        let page = self.pdf.pages().get(index).ok_or("Page out of bounds")?;

        let pixmap = hayro::render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );

        let result = js_sys::Array::new_with_length(3);
        result.set(0, JsValue::from(pixmap.width()));
        result.set(1, JsValue::from(pixmap.height()));
        result.set(2, JsValue::from(pixmap.take_rgba8(ImageAlphaType::Alpha)));

        Ok(result)
    }
}
