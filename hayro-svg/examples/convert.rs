use hayro_interpret::font::{FontData, FontQuery, StandardFont};
use hayro_interpret::{InterpreterSettings, Pdf};
use hayro_svg::convert;
use std::sync::Arc;

fn main() {
    let pdf = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let pdf = Pdf::new(Arc::new(pdf)).unwrap();

    let interpreter_settings = InterpreterSettings {
        font_resolver: Arc::new(|query| match query {
            FontQuery::Standard(s) => Some((get_standard(s), 0)),
            FontQuery::Fallback(f) => Some((get_standard(&f.pick_standard_font()), 0)),
        }),
        ..Default::default()
    };

    for (idx, page) in pdf.pages().iter().enumerate() {
        let svg = convert(page, &interpreter_settings);
        std::fs::write(format!("rendered_{idx}.svg"), svg).unwrap();
    }
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
