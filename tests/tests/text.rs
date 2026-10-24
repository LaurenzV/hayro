use crate::WORKSPACE_PATH;
use crate::harness::runner::Test;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, MarkedContentProperties, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;
use std::fmt::Write;

pub(crate) fn tests() -> Vec<Test> {
    let units: [(&str, fn()); 6] = [
        ("text::glyph_names", glyph_names),
        ("text::type3_glyph_names", type3_glyph_names),
        ("text::zapf_dingbats", zapf_dingbats),
        ("text::tex_glyph_names", tex_glyph_names),
        ("text::marked_content", marked_content),
        ("text::font_properties", font_properties),
    ];

    units.map(|(name, run)| Test::unit(name, run)).into()
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(WORKSPACE_PATH.join(path)).unwrap()
}

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
    fn begin_marked_content(&mut self, tag: &[u8], properties: &MarkedContentProperties<'_>) {
        write!(self.0, "<{}", String::from_utf8_lossy(tag)).unwrap();
        if let Some(mcid) = properties.mcid() {
            write!(self.0, " mcid={mcid}").unwrap();
        }
        if let Some(actual_text) = properties.actual_text() {
            let actual_text = String::from_utf8_lossy(actual_text.as_bytes());
            write!(self.0, " actual={actual_text}").unwrap();
        }
        self.0.push('>');
    }
    fn end_marked_content(&mut self) {
        self.0.push_str("</>");
    }
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

fn glyph_names() {
    let file = &read("pdfs/text/glyph_names.pdf");

    assert_eq!(page_text(file), "Menu: Waffles, Café & Coffee-Shop");
}

fn type3_glyph_names() {
    let file = &read("pdfs/text/type3_glyph_names.pdf");

    assert_eq!(page_text(file), "Hello, Type 3!");
}

fn zapf_dingbats() {
    let file = &read("pdfs/text/zapf_dingbats.pdf");

    assert_eq!(page_text(file), "✔ ★ ☎");
}

fn tex_glyph_names() {
    let file = &read("pdfs/text/tex_glyph_names.pdf");

    assert_eq!(page_text(file), "⌊ℓ⌋");
}

fn marked_content() {
    let file = &read("pdfs/text/marked_content.pdf");

    assert_eq!(
        page_text(file),
        "<P mcid=0>Hi</><Span mcid=1 actual=fi>x</><Artifact>.</>"
    );
}

#[derive(Default)]
struct FontCollector(Vec<String>);

impl<'a> Device<'a> for FontCollector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, glyph_run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
        let Some(font) = glyph_run.font() else {
            self.0.push("none".to_string());
            return;
        };
        self.0.push(format!(
            "{} flags={:?} weight={:?} stem_v={:?} italic_angle={:?} ascent={:?} descent={:?} cap_height={:?} bbox={:?}",
            font.base_font().map_or("?".to_string(), |name| name.as_str().to_string()),
            font.flags().map(|flags| flags.bits()),
            font.weight(),
            font.stem_v(),
            font.italic_angle(),
            font.ascent(),
            font.descent(),
            font.cap_height(),
            font.bbox().map(|bbox| (bbox.x0, bbox.y0, bbox.x1, bbox.y1)),
        ));
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn font_properties() {
    let pdf = Pdf::new(read("pdfs/text/font_properties.pdf")).unwrap();
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 1.0, 1.0),
        &cache,
        pdf.xref(),
        crate::interpreter_settings(),
    );
    let mut collector = FontCollector::default();
    interpret_page(&pdf.pages()[0], &mut context, &mut collector);

    assert_eq!(
        collector.0,
        [
            "none",
            "ABCDEF+Helvetica-Bold flags=Some(262304) weight=Some(700.0) stem_v=Some(140.0) italic_angle=Some(-12.5) ascent=Some(718.0) descent=Some(-207.0) cap_height=Some(718.0) bbox=Some((-170.0, -228.0, 1003.0, 962.0))",
            "Composite flags=Some(4) weight=None stem_v=Some(80.0) italic_angle=Some(0.0) ascent=Some(880.0) descent=Some(-120.0) cap_height=Some(700.0) bbox=Some((0.0, -120.0, 1000.0, 880.0))",
            "Helvetica flags=None weight=None stem_v=None italic_angle=None ascent=None descent=None cap_height=None bbox=None",
        ]
    );
}
