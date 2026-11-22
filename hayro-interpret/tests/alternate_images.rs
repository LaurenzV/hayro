//! Alternate image selection for display devices.

use hayro_interpret::font::GlyphRun;
use hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageData, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

#[derive(Default)]
struct Samples(Vec<(u32, Vec<u8>)>);

impl Device<'_> for Samples {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'_>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn pop_clip(&mut self) {}
    fn push_transparency_group(
        &mut self,
        _: f32,
        _: Option<SoftMask<'_>>,
        _: BlendMode,
        _: bool,
        _: bool,
    ) {
    }
    fn pop_transparency_group(&mut self) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, '_>, _: DrawProps<'_>, _: &DrawMode) {}

    fn draw_image(&mut self, image: Image<'_, '_>, _: ImageDrawProps<'_>) {
        let Image::Raster(image) = image else {
            panic!("expected a raster image")
        };
        let id = image.stream().obj_id().obj_number;
        image.with_rgba(
            |pixels, _alpha| {
                let ImageData::Rgb(rgb) = pixels else {
                    panic!("expected RGB samples")
                };
                self.0.push((id, rgb.data));
            },
            None,
        );
    }
}

#[test]
fn display_alternates_supply_the_selected_source_and_samples() {
    let pdf =
        Pdf::new(include_bytes!("../../tests/pdfs/custom/alternate_images.pdf").to_vec()).unwrap();
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 84.0, 12.0),
        &cache,
        pdf.xref(),
        InterpreterSettings::default(),
    );
    let mut samples = Samples::default();
    interpret_page(&pdf.pages()[0], &mut ctx, &mut samples);
    let red = vec![255, 0, 0];
    assert_eq!(
        samples.0,
        vec![
            (14, vec![0, 255, 0]),
            (8, red.clone()),
            (10, red.clone()),
            (11, red),
            (13, vec![0, 0, 255]),
        ]
    );
}
