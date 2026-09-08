//! Marked-content callback integration tests.

use hayro_interpret::font::GlyphRun;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, MarkedContentProperties, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;
use kurbo::{Affine, BezPath, Rect};
use pdf_writer::{Content, Name, Pdf as PdfWriter, Ref, TextStr};

#[derive(Debug, PartialEq)]
struct MarkedContentEvent {
    tag: Vec<u8>,
    mcid: Option<i32>,
    actual_text: Option<Vec<u8>>,
}

#[derive(Default)]
struct MarkedContentDevice {
    events: Vec<MarkedContentEvent>,
}

impl Device<'_> for MarkedContentDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'_>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, '_>, _: DrawProps<'_>, _: &DrawMode) {}
    fn draw_image(&mut self, _: Image<'_, '_>, _: ImageDrawProps<'_>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn begin_marked_content(&mut self, tag: &[u8], mcid: Option<i32>) {
        self.events.push(MarkedContentEvent {
            tag: tag.to_vec(),
            mcid,
            actual_text: None,
        });
    }

    fn begin_marked_content_with_properties(
        &mut self,
        tag: &[u8],
        properties: MarkedContentProperties<'_>,
    ) {
        self.events.push(MarkedContentEvent {
            tag: tag.to_vec(),
            mcid: properties.mcid(),
            actual_text: properties.actual_text().map(<[u8]>::to_vec),
        });
    }
}

#[test]
fn marked_content_exposes_inline_and_named_actual_text() {
    let pdf = Pdf::new(marked_content_pdf()).unwrap();
    let page = &pdf.pages()[0];
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        &cache,
        pdf.xref(),
        InterpreterSettings::default(),
    );
    let mut device = MarkedContentDevice::default();

    interpret_page(page, &mut context, &mut device);

    assert_eq!(
        device.events,
        [
            MarkedContentEvent {
                tag: b"Span".to_vec(),
                mcid: Some(7),
                actual_text: Some(b"Inline".to_vec()),
            },
            MarkedContentEvent {
                tag: b"Span".to_vec(),
                mcid: Some(9),
                actual_text: Some(b"Named".to_vec()),
            },
            MarkedContentEvent {
                tag: b"P".to_vec(),
                mcid: None,
                actual_text: None,
            },
        ]
    );
}

fn marked_content_pdf() -> Vec<u8> {
    let mut pdf = PdfWriter::new();
    let catalog_ref = Ref::new(1);
    let pages_ref = Ref::new(2);
    let page_ref = Ref::new(3);
    let contents_ref = Ref::new(4);

    pdf.catalog(catalog_ref).pages(pages_ref);
    pdf.pages(pages_ref).kids([page_ref]).count(1);

    {
        let mut page = pdf.page(page_ref);
        page.parent(pages_ref)
            .media_box(pdf_writer::Rect::new(0.0, 0.0, 100.0, 100.0))
            .contents(contents_ref);
        page.resources()
            .properties()
            .insert(Name(b"Named"))
            .identify(9)
            .actual_text(TextStr("Named"));
    }

    let mut content = Content::new();
    content
        .begin_marked_content_with_properties(Name(b"Span"))
        .properties()
        .identify(7)
        .actual_text(TextStr("Inline"));
    content.end_marked_content();
    content
        .begin_marked_content_with_properties(Name(b"Span"))
        .properties_named(Name(b"Named"));
    content.end_marked_content();
    content
        .begin_marked_content(Name(b"P"))
        .end_marked_content();
    pdf.stream(contents_ref, &content.finish());

    pdf.finish()
}
