//! This example shows you how you can render a PDF file to PNG.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::font::{FontData, FontQuery, StandardFont};
use hayro::hayro_interpret::hayro_cmap::CidFamily;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::{TRANSPARENT, WHITE};
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render, render_into};
use std::path::Path;
use std::sync::Arc;

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

    // This cache should be reused across multiple pages.
    let cache = RenderCache::new();
    let mut ctx = RenderContext::new(0, 0);
    let mut pixmap = Pixmap::new(0, 0);
    let mut resources = Resources::default();

    for (idx, page) in pdf.pages().iter().enumerate() {
        // hayro provides two entry points for rendering PDFs.

        // If all you need is the ability to convert a PDF page into an RGBA buffer
        // and just setting a background + scale factor is enough, use `hayro::render`:
        let settings = RenderSettings {
            x_scale: 2.0,
            y_scale: 2.0,
            bg_color: WHITE,
        };
        let rendered = render(page, &cache, &interpreter_settings, &settings);
        let output_path = format!("{}/rendered_{idx}.png", output_dir);
        std::fs::write(output_path, rendered.into_png().unwrap()).unwrap();

        // If you need/want:
        // - Support for rendering a page with arbitrary affine transforms or
        // - more control over rendering and  the ability to more effectively reuse allocations
        //   for render buffers
        // You can instead use `hayro::render_into` to directly render into a `vello_cpu`
        // `RenderContext`. However, this requires some more setup.
        // The example below shows how you can render a page zoomed 50% into the center,
        // and while reusing the `vello_cpu` `RenderContext` and `Pixmap`, which avoids
        // unnecessary reallocations when rendering multiple pages.

        // You can choose any size you desire, but in most cases you will likely want
        // to base it on the dimensions of the PDF page.
        let (width, height) = page.render_dimensions();

        // Reset the context and pixmap to the requested size.
        ctx.reset_and_resize(width as u16, height as u16);
        pixmap.resize(ctx.width(), ctx.height());

        // Zoom in by 50%, keeping the page center at the viewport center.
        let transform = Affine::translate((ctx.width() as f64 / 2.0, ctx.height() as f64 / 2.0))
            * Affine::scale(1.5)
            * Affine::translate((-width as f64 / 2.0, -height as f64 / 2.0))
            // It is important that you always add this, so that rotated/cropped
            // pages are handled correctly. Unless you know what you are doing!
            * page.initial_transform(true).to_kurbo();

        // Render into the render context.
        render_into(page, &cache, &interpreter_settings, &mut ctx, transform);

        // In case the `vello_cpu` `RenderContext` is multi-threaded, make sure to
        // flush.
        ctx.flush();

        // Finally, rasterize the scene into the pixmap. See the `vello_cpu` documentation
        // for more information!
        ctx.render_with(
            &mut pixmap,
            &mut resources,
            RasterizerSettings {
                target_init: TargetInit::Clear(TRANSPARENT),
                ..Default::default()
            },
        );

        // Encode and save the PNG.
        let output_path = format!("{}/rendered_{idx}_advanced.png", output_dir);
        std::fs::write(output_path, pixmap.clone().into_png().unwrap()).unwrap();
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
