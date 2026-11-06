use crate::WORKSPACE_PATH;
use crate::harness::runner::Test;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;
use hayro_syntax::object::{Dict, Object};
use std::fmt::Write;

pub(crate) fn tests() -> Vec<Test> {
    let units: [(&str, fn()); 8] = [
        ("text::glyph_names", glyph_names),
        ("text::type3_glyph_names", type3_glyph_names),
        ("text::zapf_dingbats", zapf_dingbats),
        ("text::tex_glyph_names", tex_glyph_names),
        ("text::marked_content", marked_content),
        ("text::clip_mode", clip_mode),
        ("text::to_unicode_cid_ranges", to_unicode_cid_ranges),
        ("text::glyph_run_font_dict", glyph_run_font_dict),
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
    fn begin_marked_content(&mut self, tag: &[u8], properties: Option<&Dict<'_>>) {
        write!(self.0, "<{}", String::from_utf8_lossy(tag)).unwrap();
        if let Some(properties) = properties {
            for key in properties.keys() {
                let value = properties.get::<Object<'_>>(&key).unwrap();
                write!(self.0, " {key} {value}").unwrap();
            }
        }
        self.0.push('>');
    }
    fn end_marked_content(&mut self) {
        self.0.push_str("</>");
    }
}

/// The character codes of each glyph run, with the object number of its font dictionary.
#[derive(Default)]
struct FontCollector(Vec<(String, Option<u32>)>);

impl<'a> Device<'a> for FontCollector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, glyph_run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
        let codes = glyph_run
            .glyphs()
            .iter()
            .filter_map(|glyph| char::from_u32(glyph.char_code()))
            .collect();
        let font = glyph_run
            .font_dict()
            .and_then(|dict| dict.obj_id())
            .map(|id| id.obj_number);
        self.0.push((codes, font));
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn interpret_first_page<D: for<'a> Device<'a> + Default>(file: &[u8]) -> D {
    let pdf = Pdf::new(file.to_vec()).unwrap();
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 1.0, 1.0),
        &cache,
        pdf.xref(),
        crate::interpreter_settings(),
    );
    let mut device = D::default();
    interpret_page(&pdf.pages()[0], &mut context, &mut device);

    device
}

fn page_text(file: &[u8]) -> String {
    interpret_first_page::<TextCollector>(file).0
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
        r#"<P /MCID 0>Hi</><Span /MCID 1 /ActualText "fi">x</><Artifact>.</>"#
    );
}

fn clip_mode() {
    let file = &read("pdfs/text/clip_mode.pdf");

    assert_eq!(page_text(file), "Clip and fill");
}

fn to_unicode_cid_ranges() {
    let file = &read("pdfs/text/to_unicode_cid_ranges.pdf");

    assert_eq!(page_text(file), "HI");
}

fn glyph_run_font_dict() {
    let file = &read("pdfs/text/glyph_run_font_dict.pdf");

    // `b` and `c` are shown with two Helvetica dictionaries that share a program. `a` is
    // shown before a font is selected, `d` with a font that can't be loaded and `e` with
    // one that doesn't exist, so all three use the fallback font.
    assert_eq!(
        interpret_first_page::<FontCollector>(file).0,
        [
            ("a".to_string(), None),
            ("b".to_string(), Some(5)),
            ("c".to_string(), Some(6)),
            ("d".to_string(), None),
            ("e".to_string(), None),
        ]
    );
}
