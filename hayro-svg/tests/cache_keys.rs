//! SVG cache keys.

use hayro_interpret::InterpreterSettings;
use hayro_svg::hayro_syntax::Pdf;
use hayro_svg::{RenderCache, SvgRenderSettings, convert};

fn svg(bytes: &[u8]) -> String {
    let pdf = Pdf::new(bytes.to_vec()).unwrap();
    convert(
        &pdf.pages()[0],
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &SvgRenderSettings::default(),
    )
}

#[test]
fn type3_transfer_functions() {
    let svg = svg(include_bytes!("fixtures/glyph_transfer.pdf"));
    assert_eq!(svg.matches("<g id=\"e").count(), 2);
    assert!(svg.contains("stop-color=\"#808080\""));
    assert!(svg.contains("stop-color=\"#404040\""));
}

fn definitions<'a>(svg: &'a str, id: &str) -> &'a str {
    let (_, section) = svg.split_once(&format!("<defs id=\"{id}\">")).unwrap();
    section.split_once("</defs>").unwrap().0
}

#[test]
fn shallow_type3_after_depth_limit() {
    let svg = svg(include_bytes!("fixtures/glyph_mask_depth.pdf"));
    let glyphs = definitions(&svg, "type3-glyph");
    assert!(glyphs.contains("<g id=\"e0\"/>"));
    assert!(glyphs.contains("<g id=\"e1\">"));
    assert_eq!(glyphs.matches("<rect ").count(), 1);
    assert!(svg.contains("xlink:href=\"#e1\""));
}

#[test]
fn shallow_soft_mask_after_depth_limit() {
    let svg = svg(include_bytes!("fixtures/glyph_mask_depth.pdf"));
    let masks = definitions(&svg, "mask");
    let (deep, shallow) = masks.split_once("<mask id=\"m1\"").unwrap();
    assert!(deep.contains("<mask id=\"m0\""));
    assert!(!deep.contains("<rect "));
    assert!(shallow.contains("<rect "));
    assert!(svg.contains("mask=\"url(#m1)\""));
}
