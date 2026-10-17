use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

#[derive(Default)]
struct TextCollector(String);

impl<'a> Device<'a> for TextCollector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, glyph_run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
        for glyph in glyph_run.glyphs() {
            match glyph.as_unicode() {
                Some(BfString::Char(c)) => self.0.push(c),
                Some(BfString::String(s)) => self.0.push_str(&s),
                None => self.0.push(char::REPLACEMENT_CHARACTER),
            }
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn page_text(file: &[u8]) -> String {
    let pdf = Pdf::new(file.to_vec()).unwrap();
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 1.0, 1.0),
        &cache,
        pdf.xref(),
        crate::interpreter_settings(),
    );
    let mut collector = TextCollector::default();
    interpret_page(&pdf.pages()[0], &mut context, &mut collector);

    collector.0
}

#[test]
fn glyph_names() {
    let file = include_bytes!("../pdfs/text/glyph_names.pdf");

    assert_eq!(page_text(file), "Menu: Waffles, Café & Coffee-Shop");
}

#[test]
fn type3_glyph_names() {
    let file = include_bytes!("../pdfs/text/type3_glyph_names.pdf");

    assert_eq!(page_text(file), "Hello, Type 3!");
}
