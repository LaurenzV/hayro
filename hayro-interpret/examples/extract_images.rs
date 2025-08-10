//! This example demonstrates how you can extract all images used on a page and save them as
//! PNG.
//!
//! Note that you must have downloaded the corresponding PDF file for the example to work.

use hayro_interpret::font::Glyph;
use hayro_interpret::{
    ClipPath, Context, Device, FillRule, InterpreterSettings, LumaData, Paint, RgbData, SoftMask,
    StrokeProps, interpret_page,
};
use hayro_syntax::Pdf;
use image::{DynamicImage, ImageBuffer};
use kurbo::{Affine, BezPath, Rect};
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let data = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro-tests/pdfs/image_rgb8.pdf"),
    )
    .unwrap();

    let pdf = Pdf::new(Arc::new(data)).unwrap();

    let mut extractor = ImageExtractor::new();
    let settings = InterpreterSettings::default();
    // Pass dummy values for bbox and initial transform, since we don't care about those.
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 1.0, 1.0),
        pdf.xref(),
        settings,
    );

    // Run everything!
    let page = &pdf.pages()[0];
    interpret_page(page, &mut context, &mut extractor);

    // Then just save all of the images!
    for (idx, img) in extractor.0.iter().enumerate() {
        img.save(format!("image_{}.png", idx)).unwrap();
    }
}

struct ImageExtractor(Vec<DynamicImage>);

impl ImageExtractor {
    fn new() -> Self {
        Self(Vec::new())
    }
}

/// Implement `Device` for `ImageExtractor`. We can ignore most operations and only
/// need to implement `draw_rgba_image` and `draw_stencil_image`.
impl Device<'_> for ImageExtractor {
    fn stroke_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &StrokeProps) {}

    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}

    fn fill_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: FillRule) {}

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>) {}

    fn fill_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>) {}

    fn stroke_glyph(
        &mut self,
        _: &Glyph<'_>,
        _: Affine,
        _: Affine,
        _: &Paint<'_>,
        _: &StrokeProps,
    ) {
    }

    // The alpha and RGB channels are provided separately.
    fn draw_rgba_image(&mut self, image: RgbData, _: Affine, alpha: Option<LumaData>) {
        let image = if let Some(alpha) = alpha {
            // This is not complete, as it can in theory happen that the alpha channel has a different
            // dimension than the RGB channel. We ignore this edge case for this example.
            if alpha.width == image.width && alpha.height == image.height {
                let interleaved = image
                    .data
                    .chunks(3)
                    .zip(alpha.data)
                    .flat_map(|(rgb, a)| [rgb[0], rgb[1], rgb[2], a])
                    .collect::<Vec<u8>>();

                DynamicImage::ImageRgba8(
                    ImageBuffer::from_raw(image.width, image.height, interleaved).unwrap(),
                )
            } else {
                DynamicImage::ImageRgb8(
                    ImageBuffer::from_raw(image.width, image.height, image.data.clone()).unwrap(),
                )
            }
        } else {
            DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(image.width, image.height, image.data.clone()).unwrap(),
            )
        };

        self.0.push(image);
    }

    // Stencil images are grey-channel images that should be painted using the color stored in
    // `paint`. For simplicity, we just store them as grey-channel for now.
    fn draw_stencil_image(&mut self, stencil: LumaData, _: Affine, _: &Paint<'_>) {
        self.0.push(DynamicImage::ImageLuma8(
            ImageBuffer::from_raw(stencil.width, stencil.height, stencil.data.clone()).unwrap(),
        ))
    }

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}
