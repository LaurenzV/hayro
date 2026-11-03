use crate::harness::runner::Test;
use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::pattern::Pattern;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, ContentSource, Context, Device, DrawMode, DrawProps, Image,
    ImageDrawProps, InterpreterCache, Paint, SoftMask, interpret_page,
};
use hayro_syntax::object::Stream;
use hayro_syntax::xref::XRef;
use std::ops::Range;

pub(crate) fn tests() -> Vec<Test> {
    vec![Test::unit("interpret::content_sources", content_sources)]
}

/// Records the content streams and instructions that the interpreter reports. Like a renderer,
/// it interprets tiling patterns, Type 3 glyphs and soft masks when they are drawn.
struct ContentRecorder<'x> {
    xref: &'x XRef,
    /// The decoded data of the content streams being interpreted, starting with the page's.
    streams: Vec<Vec<u8>>,
    log: Vec<String>,
}

impl<'a> Device<'a> for ContentRecorder<'_> {
    fn draw_path(&mut self, _: &BezPath, props: DrawProps<'a>, _: &DrawMode) {
        if let Paint::Pattern(pattern) = &props.paint
            && let Pattern::Tiling(tiling) = pattern.as_ref()
        {
            tiling.interpret(self, Affine::IDENTITY, false);
        }

        if let Some(mask) = &props.soft_mask {
            mask.interpret(self);
        }
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, glyph_run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for glyph in glyph_run.glyphs() {
            if let Glyph::Type3(type3) = &**glyph {
                type3.interpret(self, props.transform, glyph.transform(), &props.paint);
            }
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn begin_content(&mut self, source: &ContentSource<'_>) {
        let (entry, stream) = match source {
            ContentSource::FormXObject { name, stream } => {
                (format!("<form {name} {}>", stream.obj_number), stream)
            }
            ContentSource::Appearance { annotation, stream } => (
                format!(
                    "<appearance {} {}>",
                    annotation.unwrap().obj_number,
                    stream.obj_number
                ),
                stream,
            ),
            ContentSource::TilingPattern { name, stream } => {
                (format!("<pattern {name} {}>", stream.obj_number), stream)
            }
            ContentSource::Type3Glyph { font, code, stream } => (
                format!(
                    "<glyph {} {code} {}>",
                    font.unwrap().obj_number,
                    stream.obj_number
                ),
                stream,
            ),
            ContentSource::SoftMask { stream } => (format!("<mask {}>", stream.obj_number), stream),
            _ => unreachable!(),
        };
        self.log.push(entry);

        let stream = self.xref.get::<Stream<'_>>(*stream).unwrap();
        let data = stream.decoded_data().unwrap().into_owned();
        self.streams.push(data);
    }
    fn end_content(&mut self) {
        self.streams.pop();
        self.log.push("</>".to_string());
    }
    fn operator(&mut self, span: Range<usize>) {
        let instruction = String::from_utf8_lossy(&self.streams.last().unwrap()[span]);
        self.log.push(instruction.into_owned());
    }
}

fn content_sources() {
    let pdf = crate::load_pdf("pdfs/interpret/content_sources.pdf");
    let page = &pdf.pages()[0];
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        &cache,
        pdf.xref(),
        crate::interpreter_settings(),
    );
    let mut recorder = ContentRecorder {
        xref: pdf.xref(),
        streams: vec![page.page_stream().unwrap().to_vec()],
        log: vec![],
    };
    interpret_page(page, &mut context, &mut recorder);

    // Each instruction is the range reported for it, taken from the stream the innermost
    // content source names.
    assert_eq!(
        recorder.log,
        [
            "/Fm0 Do",
            "<form /Fm0 5>",
            "0 0 1 rg",
            "/Fm1 Do",
            "<form /Fm1 6>",
            "0 0 10 10 re",
            "f",
            "</>",
            "</>",
            "q",
            "/Cs0 cs",
            "1 0 0 /P0 scn",
            "0 0 20 20 re",
            "f",
            "<pattern /P0 7>",
            "0 0 5 5 re",
            "f",
            "</>",
            "Q",
            "BT",
            "/F1 12 Tf",
            "30 0 Td",
            "(a) Tj",
            "<glyph 8 97 9>",
            "10 0 0 0 10 10 d1",
            "0 0 10 10 re",
            "f",
            "</>",
            "ET",
            "/GS0 gs",
            "50 0 20 20 re",
            "f",
            "<mask 10>",
            "0 0 100 100 re",
            "f",
            "</>",
            "<appearance 11 12>",
            "0 0 20 20 re",
            "S",
            "</>",
        ]
    );
}
