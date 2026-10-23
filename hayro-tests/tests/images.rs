use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

#[derive(Default)]
struct StencilCollector(Vec<(i32, Vec<u8>)>);

impl<'a> Device<'a> for StencilCollector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}
    fn draw_image(&mut self, image: Image<'a, '_>, _: ImageDrawProps<'a>) {
        if let Image::Stencil(stencil) = image {
            let stream = stencil.stream();
            // The data of inline images includes the white space before `EI`.
            let data = stream.raw_data().trim_ascii_end().to_vec();
            self.0.push((stream.obj_id().obj_number, data));
        }
    }
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn stencil_streams(file: &[u8]) -> Vec<(i32, Vec<u8>)> {
    let pdf = Pdf::new(file.to_vec()).unwrap();
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 1.0, 1.0),
        &cache,
        pdf.xref(),
        crate::interpreter_settings(),
    );
    let mut collector = StencilCollector::default();
    interpret_page(&pdf.pages()[0], &mut context, &mut collector);

    collector.0
}

#[test]
fn stencil_stream() {
    let file = include_bytes!("../pdfs/images/stencil_masks.pdf");

    // The image mask XObject (object 5) and the inline image mask.
    assert_eq!(
        stencil_streams(file),
        [(5, vec![0x0f, 0xf0]), (0, vec![0xf0, 0x0f])]
    );
}
