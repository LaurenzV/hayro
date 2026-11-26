use crate::Renderer;
use hayro_interpret::Paint;
use hayro_interpret::encode::{EncodedShadingPattern, EncodedShadingType, texture_dimensions};
use hayro_interpret::pattern::Pattern;
use hayro_interpret::util::x_y_advances;
use kurbo::{Affine, BezPath, Rect};
use std::sync::Arc;
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::peniko::{ImageQuality, ImageSampler};
use vello_cpu::{Image, ImageSource, PaintType, PixelMetadata, Pixmap, peniko};

impl Renderer<'_> {
    #[must_use]
    pub(super) fn set_paint(
        &mut self,
        paint: &Paint<'_>,
        path_bbox: impl Fn() -> Rect,
        is_stroke: bool,
    ) -> Option<BezPath> {
        let mut paint_transform = Affine::IDENTITY;
        let mut clip_path = None;

        let paint: PaintType = match paint.clone() {
            Paint::Color(c) => {
                let c = c.to_rgba().to_rgba8();
                AlphaColor::from_rgba8(c[0], c[1], c[2], c[3]).into()
            }
            Paint::Pattern(p) => {
                let path_transform = self.ctx.transform();
                let bbox = {
                    let mut bbox = path_transform.transform_rect_bbox(path_bbox());

                    if is_stroke {
                        let (a1, a2) = x_y_advances(path_transform);
                        let factor = a1.length().max(a2.length()) * self.ctx.stroke().width;
                        bbox = bbox.inflate(factor, factor);
                    }

                    bbox.intersect(Rect::new(
                        0.0,
                        0.0,
                        self.ctx.width() as f64,
                        self.ctx.height() as f64,
                    ))
                };

                match *p {
                    Pattern::Shading(s) => {
                        clip_path = s.shading.clip_path.clone();
                        let encoded = s.encode();
                        let bbox = bbox.expand();
                        let bbox = match &encoded.shading_type {
                            EncodedShadingType::RadialAxial(g) => g.texture_bounds(bbox, &encoded),
                            _ => bbox,
                        };
                        let (paint, transform) = texture_paint(&encoded, bbox);
                        paint_transform = path_transform.inverse() * transform;
                        paint
                    }
                    Pattern::Tiling(t) => {
                        let (image, transform) = self.tiling_paint(&t, bbox, is_stroke);
                        paint_transform = path_transform.inverse() * transform;

                        PaintType::Image(image)
                    }
                }
            }
        };

        self.ctx.set_paint_transform(paint_transform);
        self.ctx.set_paint(paint);

        clip_path
    }
}

fn texture_paint(encoded: &EncodedShadingPattern, bbox: Rect) -> (PaintType, Affine) {
    let (width, height) = texture_dimensions(bbox, 1.0);
    let mut image = Vec::with_capacity(width as usize * height as usize);
    let mut may_have_transparency = false;
    let (width, height, transform) = encoded.sample_texture(bbox, 1.0, |sample| {
        let pixel = AlphaColor::<Srgb>::new(sample).premultiply().to_rgba8();
        may_have_transparency |= pixel.a != 255;
        image.push(pixel);
    });

    let pixmap = Pixmap::from_parts(
        bytemuck::cast_vec(image),
        width as u16,
        height as u16,
        PixelMetadata {
            may_have_transparency,
            ..Default::default()
        },
    );

    let image = Image {
        image: ImageSource::Pixmap(Arc::new(pixmap)),
        sampler: ImageSampler {
            x_extend: peniko::Extend::Pad,
            y_extend: peniko::Extend::Pad,
            quality: ImageQuality::Medium,
            alpha: 1.0,
        },
    };

    (PaintType::Image(image), transform)
}
