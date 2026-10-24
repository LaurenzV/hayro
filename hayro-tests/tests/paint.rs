use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, Paint, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

#[derive(Default)]
struct ColorCollector(Vec<(u8, Vec<f32>)>);

impl<'a> Device<'a> for ColorCollector {
    fn draw_path(&mut self, _: &BezPath, props: DrawProps<'a>, _: &DrawMode) {
        if let Paint::Color(color) = props.paint {
            self.0.push((
                color.color_space().num_components(),
                color.components().to_vec(),
            ));
        }
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn fill_colors(file: &[u8]) -> Vec<(u8, Vec<f32>)> {
    let pdf = Pdf::new(file.to_vec()).unwrap();
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 1.0, 1.0),
        &cache,
        pdf.xref(),
        crate::interpreter_settings(),
    );
    let mut collector = ColorCollector::default();
    interpret_page(&pdf.pages()[0], &mut context, &mut collector);

    collector.0
}

#[test]
fn fill_colors_components() {
    let file = include_bytes!("../pdfs/paint/fill_colors.pdf");

    assert_eq!(
        fill_colors(file),
        [
            (1, vec![0.25]),
            (4, vec![0.1, 0.2, 0.3, 0.4]),
            (1, vec![0.5]),
            (2, vec![0.5, 0.75]),
        ]
    );
}
