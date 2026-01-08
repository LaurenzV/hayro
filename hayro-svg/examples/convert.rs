//! Convert a PDF file into a series of SVG files.

use hayro_interpret::InterpreterSettings;
use hayro_svg::convert;
use hayro_svg::hayro_syntax::Pdf;
use std::sync::Arc;

fn main() {
    let pdf = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let pdf = Pdf::new(Arc::new(pdf)).unwrap();

    let interpreter_settings = InterpreterSettings::default();

    for (idx, page) in pdf.pages().iter().enumerate() {
        let svg = convert(page, &interpreter_settings);
        std::fs::write(format!("rendered_{idx}.svg"), svg).unwrap();
    }
}
