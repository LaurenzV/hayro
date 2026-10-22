//! A page content under a predictor whose last row is cut short is drawn whole, as
//! viewers draw it.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

#[path = "../../hayro-syntax/tests/common/mod.rs"]
#[allow(dead_code)]
mod common;

/// The `f` alone in the content's last row fills the blue square, as `PyMuPDF` 1.28.2
/// renders it (pixel (30, 70) of 100 × 100 is blue).
#[test]
fn a_content_whose_last_row_is_cut_short_is_drawn() {
    let pdf = Pdf::new(common::one_page(
        100,
        "/Contents 4 0 R",
        vec![common::cut_row_content_stream()],
    ))
    .unwrap();
    let pixmap = render(
        &pdf.pages()[0],
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: 1.0,
            y_scale: 1.0,
            bg_color: WHITE,
        },
    );
    let p = pixmap.sample(30, 70);
    assert_eq!([p.r, p.g, p.b], [0, 0, 255]);
}
