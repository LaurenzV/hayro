use crate::Id;
use crate::{SvgRenderer, convert_transform};
use hayro_interpret::encode::EncodedShadingPattern;
use hayro_interpret::pattern::{Pattern, ShadingPattern, TilingPattern};
use hayro_interpret::{CacheKey, Paint};
use image::{DynamicImage, ImageBuffer};
use kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};

#[derive(Clone)]
pub(crate) struct CachedTilingPattern<'a> {
    pub(crate) transform: Affine,
    pub(crate) tiling_pattern: TilingPattern<'a>,
}

pub(crate) struct CachedShadingPattern {
    pub(crate) transform: Affine,
    pub(crate) shading: Id,
    pub(crate) bbox: Rect,
}

pub(crate) struct CachedShading {
    pub(crate) pattern: ShadingPattern,
    pub(crate) bbox: Rect,
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn write_paint(
        &mut self,
        paint: &Paint<'a>,
        path: &BezPath,
        path_transform: Affine,
        is_stroke: bool,
    ) {
        let (paint_str, alpha) = match &paint {
            Paint::Color(c) => {
                let rgba8 = c.to_rgba().to_rgba8();
                let color = format!(
                    "#{}",
                    &rgba8[0..3]
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                );
                let alpha = rgba8[3] as f32 / 255.0;

                (color, alpha)
            }
            Paint::Pattern(p) => {
                let id = match p.as_ref() {
                    Pattern::Shading(s) => {
                        let bbox = (path_transform * path).bounding_box();
                        let shading_id =
                            self.shadings.insert_with(s.cache_key(), || CachedShading {
                                pattern: s.clone(),
                                bbox,
                            });

                        let inverse_transform = path_transform.inverse();

                        self.shading_patterns.insert_with(
                            (s.clone(), inverse_transform).cache_key(),
                            || CachedShadingPattern {
                                transform: inverse_transform,
                                bbox,
                                shading: shading_id,
                            },
                        )
                    }
                    Pattern::Tiling(t) => {
                        let inverse_transform = path_transform.inverse();
                        let pattern = *t.clone();
                        let cache_key = (pattern.clone(), inverse_transform).cache_key();

                        if !self.tiling_patterns.contains(cache_key) {
                            self.with_dummy(|r| {
                                t.interpret(r, Affine::IDENTITY, false);
                            })
                        }

                        self.tiling_patterns
                            .insert_with(cache_key, || CachedTilingPattern {
                                transform: inverse_transform,
                                tiling_pattern: pattern,
                            })
                    }
                };

                (format!("url(#{id})"), 1.0)
            }
        };

        if is_stroke {
            self.xml.write_attribute("fill", "none");
            self.xml.write_attribute("stroke", &paint_str);
            if alpha != 1.0 {
                self.xml.write_attribute("stroke-opacity", &alpha);
            }
        } else {
            self.xml.write_attribute("fill", &paint_str);

            if alpha != 1.0 {
                self.xml.write_attribute("fill-opacity", &alpha);
            }
        }
    }

    pub(crate) fn write_shading_pattern_defs(&mut self) {
        if self.shading_patterns.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "shading-pattern");

        for (id, shading) in self.shading_patterns.iter() {
            self.xml.start_element("pattern");
            self.xml.write_attribute("id", &id);
            self.xml.write_attribute("patternUnits", "userSpaceOnUse");
            self.xml.write_attribute("width", &shading.bbox.x1);
            self.xml.write_attribute("height", &shading.bbox.y1);
            self.xml.write_attribute(
                "patternTransform",
                &format!("matrix({})", convert_transform(&shading.transform)),
            );

            self.xml.start_element("use");
            self.xml
                .write_attribute("xlink:href", &format!("#{}", shading.shading));
            self.xml.end_element();

            self.xml.end_element();
        }

        self.xml.end_element();
    }

    pub(crate) fn write_tiling_pattern_defs(&mut self) {
        if self.tiling_patterns.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "tiling-pattern");

        let patterns = self
            .tiling_patterns
            .iter()
            .map(|i| (i.0, i.1.clone()))
            .collect::<Vec<_>>();

        for (id, pattern) in patterns {
            let pattern = pattern.clone();
            let transform = pattern.transform * pattern.tiling_pattern.matrix;

            self.xml.start_element("pattern");
            self.xml.write_attribute("id", &id);
            self.xml.write_attribute("patternUnits", "userSpaceOnUse");
            self.xml
                .write_attribute("width", &pattern.tiling_pattern.x_step);
            self.xml
                .write_attribute("height", &pattern.tiling_pattern.y_step);
            self.xml.write_attribute(
                "patternTransform",
                &format!("matrix({})", convert_transform(&transform)),
            );

            // TODO: Write bbox
            pattern
                .tiling_pattern
                .interpret(self, Affine::IDENTITY, false);

            self.xml.end_element();
        }

        self.xml.end_element();
    }

    pub(crate) fn write_shading_defs(&mut self) {
        if self.shadings.is_empty() {
            return;
        }

        let shadings = std::mem::take(&mut self.shadings);

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "shading");

        for (id, shading) in shadings.iter() {
            let encoded = shading.pattern.encode();
            let (image, transform) = render_shading_texture(shading.bbox, &encoded);
            self.write_image(&image, true, Some(id), transform);
        }

        self.xml.end_element();
    }
}

fn render_shading_texture(
    bbox: Rect,
    shading_pattern: &EncodedShadingPattern,
) -> (DynamicImage, Affine) {
    const SCALE: f32 = 2.0;
    const INV_SCALE: f32 = 1.0 / SCALE;

    let base_width = bbox.width() as f32;
    let base_height = bbox.height() as f32;

    let width = (base_width * SCALE).ceil() as u32;
    let height = (base_height * SCALE).ceil() as u32;

    let initial_transform = Affine::scale(INV_SCALE as f64)
        * shading_pattern.base_transform
        * Affine::translate((0.5, 0.5));
    let (x_advance, y_advance) = x_y_advances(&initial_transform);

    let mut buf = vec![0u8; width as usize * height as usize * 4];
    let mut start_point = initial_transform * Point::new(bbox.x0, bbox.y0);

    for row in buf.chunks_exact_mut(width as usize * 4) {
        let mut point = start_point;

        for pixel in row.chunks_exact_mut(4) {
            let sample = shading_pattern.sample(point);
            let converted = [
                (sample[0] * 255.0 + 0.5) as u8,
                (sample[1] * 255.0 + 0.5) as u8,
                (sample[2] * 255.0 + 0.5) as u8,
                (sample[3] * 255.0 + 0.5) as u8,
            ];

            pixel.copy_from_slice(&converted);

            point += x_advance;
        }

        start_point += y_advance;
    }

    let image = DynamicImage::ImageRgba8(ImageBuffer::from_raw(width, height, buf).unwrap());

    (
        image,
        Affine::translate((bbox.x0, bbox.y0)) * Affine::scale(INV_SCALE as f64),
    )
}

fn x_y_advances(transform: &Affine) -> (Vec2, Vec2) {
    let scale_skew_transform = {
        let c = transform.as_coeffs();
        Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
    };

    let x_advance = scale_skew_transform * Point::new(1.0, 0.0);
    let y_advance = scale_skew_transform * Point::new(0.0, 1.0);

    (
        Vec2::new(x_advance.x, x_advance.y),
        Vec2::new(y_advance.x, y_advance.y),
    )
}
