//! Pattern paint coverage for stroked paths.

use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{
    PixmapSettings, RenderCache, RenderSettings, hayro_interpret::InterpreterSettings,
    hayro_syntax::Pdf, render,
};

#[test]
fn tiling_pattern_paints_acute_miter_tip() {
    let pdf = Pdf::new(include_bytes!("fixtures/pattern-miter.pdf").to_vec()).unwrap();
    let image = render_page(&pdf);
    let pixel = image.data()[33 * 128 + 64];
    assert!(
        pixel.b > pixel.r && pixel.r < 64,
        "miter tip pixel {pixel:?}"
    );
}

fn render_page(pdf: &Pdf) -> vello_cpu::Pixmap {
    render(
        &pdf.pages()[0],
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            bg_color: WHITE,
            ..PixmapSettings::default()
        },
    )
}

fn patterned_stroke(commands: &str) -> Pdf {
    patterned_stroke_with_cell(commands, 32, "0 0 1 rg 13 0 19 32 re f")
}

fn patterned_stroke_with_cell(commands: &str, size: u16, cell: &str) -> Pdf {
    let stream = format!("/Pattern CS /P SCN 2 w {commands}");
    let bytes = format!("%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Count 1 /Kids [3 0 R] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 128 128] /Resources << /Pattern << /P 5 0 R >> >> /Contents 4 0 R >> endobj
4 0 obj << /Length {} >>
stream
{stream}
endstream
endobj
5 0 obj << /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 {size} {size}] /Matrix [1 0 0 1 .75 0] /XStep {size} /YStep {size} /Resources << >> /Length {} >>
stream
{cell}
endstream
endobj
trailer << /Root 1 0 R >>
%%EOF", stream.len(), cell.len());
    Pdf::new(bytes.into_bytes()).unwrap()
}

#[test]
fn negative_miter_matches_positive() {
    let fixture = include_bytes!("fixtures/pattern-miter.pdf");
    let mut negative = fixture.to_vec();
    let start = negative.windows(8).position(|w| w == b"0 j 10 M").unwrap();
    negative[start..start + 8].copy_from_slice(b"   -10 M");
    assert_eq!(
        render_page(&Pdf::new(negative).unwrap()).data(),
        render_page(&Pdf::new(fixture.to_vec()).unwrap()).data()
    );
}

#[test]
fn cropped_tile_retains_bilinear_neighbor() {
    let pdf = patterned_stroke("2 j 12.75 10 m 12.75 20 l S");
    let pixel = render_page(&pdf).data()[113 * 128 + 13];
    assert!(pixel.b == 255 && pixel.r < 255 && pixel.r > 0, "{pixel:?}");
}

#[test]
fn high_miter_limit_keeps_small_pattern_textures() {
    use hayro::{
        kurbo::Affine,
        render_into,
        vello_cpu::{ImageSource, PaintType, RenderContext},
    };
    for (path, size) in [
        ("16 10 m 16 20 l S", (5, 14)),
        ("16 10 m 16 20 l 24 20 l S", (14, 15)),
    ] {
        let pdf = patterned_stroke(&format!("10000 M {path}"));
        let mut ctx = RenderContext::new(128, 128);
        render_into(
            &pdf.pages()[0],
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &mut ctx,
            Affine::IDENTITY,
        );
        let PaintType::Image(image) = ctx.paint() else {
            panic!("expected a pattern texture")
        };
        let ImageSource::Pixmap(texture) = &image.image else {
            panic!("expected a pixmap")
        };
        assert_eq!((texture.width(), texture.height()), size);
        assert_eq!(
            render_page(&pdf).data(),
            render_page(&patterned_stroke(path)).data()
        );
    }
}

#[test]
fn closing_and_curved_joins_paint_acute_tips() {
    for path in [
        "60 80 m 76 20 l 44 20 l h S",
        "44 20 m 42 18 60 80 60 80 c 60 80 76 20 78 22 c S",
    ] {
        let pdf = patterned_stroke(&format!("10 w 0 j 10 M {path}"));
        let pixel = render_page(&pdf).data()[33 * 128 + 60];
        assert!(pixel.b > pixel.r && pixel.r < 64, "{path}: {pixel:?}");
    }
}

#[test]
fn skewed_miter_samples_pattern_at_tip() {
    let pdf = patterned_stroke_with_cell(
        "1 .4 1 -.4 80 64 cm 20 w 0 j 10 M -8 -32 m 0 0 l -32 -8 l S",
        128,
        "1 0 0 rg 0 0 128 128 re f 0 0 1 rg 103 0 25 128 re f",
    );
    let pixel = render_page(&pdf).data()[64 * 128 + 104];
    assert!(
        pixel.b == 255 && pixel.r == pixel.g && pixel.r < 128,
        "{pixel:?}"
    );
}
