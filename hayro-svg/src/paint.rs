use crate::clip::CachedClipPath;
use crate::{Id, hash128};
use crate::{SvgRenderer, convert_transform};
use hayro_interpret::encode::{EncodedShadingPattern, EncodedShadingType};
use hayro_interpret::gradient::{SvgGradient, SvgGradientKind};
use hayro_interpret::pattern::{Pattern, ShadingPattern, TilingPattern};
use hayro_interpret::{CacheKey, FillRule, Paint, StrokeProps};
use image::{DynamicImage, ImageBuffer};
use kurbo::{Affine, Point, Rect, Shape, Vec2};

#[derive(Clone)]
pub(crate) struct CachedTilingPattern<'a> {
    pub(crate) transform: Affine,
    pub(crate) tiling_pattern: TilingPattern<'a>,
}

pub(crate) struct CachedShadingPattern {
    pub(crate) transform: Affine,
    pub(crate) paint: CachedShadingPaint,
    pub(crate) clip_path: Option<Id>,
    pub(crate) bbox: Rect,
}

pub(crate) enum CachedShadingPaint {
    Raster { shading: Id },
    NativeGradient { gradient: Id },
}

pub(crate) struct CachedNativeGradient {
    pub(crate) gradient: SvgGradient,
}

pub(crate) struct CachedShading {
    pub(crate) pattern: ShadingPattern,
    pub(crate) bbox: Rect,
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn write_paint(
        &mut self,
        paint: &Paint<'a>,
        path_bbox: impl Fn() -> Rect,
        path_transform: Affine,
        stroke_props: Option<&StrokeProps>,
    ) {
        let (paint_str, alpha) = self.svg_paint(paint, &path_bbox, path_transform, stroke_props);

        if stroke_props.is_some() {
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

    pub(crate) fn write_fill_and_stroke_paint(
        &mut self,
        paint: &Paint<'a>,
        path_bbox: impl Fn() -> Rect,
        path_transform: Affine,
        stroke_props: &StrokeProps,
    ) {
        let (fill_str, fill_alpha) = self.svg_paint(paint, &path_bbox, path_transform, None);
        let (stroke_str, stroke_alpha) =
            self.svg_paint(paint, &path_bbox, path_transform, Some(stroke_props));

        self.xml.write_attribute("fill", &fill_str);
        self.xml.write_attribute("stroke", &stroke_str);

        if fill_alpha != 1.0 {
            self.xml.write_attribute("fill-opacity", &fill_alpha);
        }

        if stroke_alpha != 1.0 {
            self.xml.write_attribute("stroke-opacity", &stroke_alpha);
        }
    }

    fn svg_paint(
        &mut self,
        paint: &Paint<'a>,
        path_bbox: &impl Fn() -> Rect,
        path_transform: Affine,
        stroke_props: Option<&StrokeProps>,
    ) -> (String, f32) {
        match &paint {
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
                        const NATIVE_GRADIENT_TOLERANCE: f32 = 0.01;

                        let mut basic_bbox = path_bbox();

                        if let Some(stroke_width) = stroke_props.map(|s| s.line_width) {
                            basic_bbox =
                                basic_bbox.inflate(stroke_width as f64, stroke_width as f64);
                        }

                        let bbox = (path_transform * basic_bbox.to_path(0.0)).bounding_box();
                        let shading_key = hash128(&(
                            s.cache_key(),
                            bbox.x0.to_bits(),
                            bbox.x1.to_bits(),
                            bbox.y0.to_bits(),
                            bbox.y1.to_bits(),
                        ));

                        let encoded = s.encode();
                        let shading_paint = if let EncodedShadingType::RadialAxial(gradient) =
                            &encoded.shading_type
                            && let Some(native) =
                                gradient.as_svg_gradient(&encoded, bbox, NATIVE_GRADIENT_TOLERANCE)
                        {
                            let gradient_key = gradient_key(shading_key, &native);
                            let gradient_id =
                                self.gradients
                                    .insert_with(gradient_key, || CachedNativeGradient {
                                        gradient: native,
                                    });

                            CachedShadingPaint::NativeGradient {
                                gradient: gradient_id,
                            }
                        } else {
                            let shading_id =
                                self.shadings.insert_with(shading_key, || CachedShading {
                                    pattern: s.clone(),
                                    bbox,
                                });

                            CachedShadingPaint::Raster {
                                shading: shading_id,
                            }
                        };

                        let clip_path = s.shading.clip_path.clone().map(|path| {
                            self.clip_paths.insert(CachedClipPath::Path {
                                path,
                                fill_rule: FillRule::NonZero,
                            })
                        });

                        let inverse_transform = path_transform.inverse();

                        let pattern_key = (shading_key, inverse_transform).cache_key();

                        self.shading_patterns
                            .insert_with(pattern_key, || CachedShadingPattern {
                                transform: inverse_transform,
                                bbox,
                                clip_path,
                                paint: shading_paint,
                            })
                    }
                    Pattern::Tiling(t) => {
                        let inverse_transform = path_transform.inverse();
                        let pattern = *t.clone();
                        let cache_key = (pattern.clone(), inverse_transform).cache_key();

                        if !self.tiling_patterns.contains(cache_key) {
                            self.with_dummy(|r| {
                                t.interpret(
                                    r,
                                    Affine::translate((-pattern.bbox.x0, -pattern.bbox.y0)),
                                    false,
                                );
                            });
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

            match &shading.paint {
                CachedShadingPaint::Raster {
                    shading: shading_id,
                } => {
                    self.xml.start_element("use");
                    if let Some(clip) = shading.clip_path {
                        self.xml
                            .write_attribute_fmt("clip-path", format_args!("url(#{clip})"));
                    }
                    self.xml
                        .write_attribute("xlink:href", &format!("#{shading_id}"));
                    self.xml.end_element();
                }
                CachedShadingPaint::NativeGradient { gradient } => {
                    self.xml.start_element("rect");
                    self.xml.write_attribute("x", &shading.bbox.x0);
                    self.xml.write_attribute("y", &shading.bbox.y0);
                    self.xml.write_attribute("width", &shading.bbox.width());
                    self.xml.write_attribute("height", &shading.bbox.height());
                    if let Some(clip) = shading.clip_path {
                        self.xml
                            .write_attribute_fmt("clip-path", format_args!("url(#{clip})"));
                    }
                    self.xml
                        .write_attribute("fill", &format!("url(#{gradient})"));
                    self.xml.end_element();
                }
            }

            self.xml.end_element();
        }

        self.xml.end_element();
    }

    pub(crate) fn write_native_gradient_defs(&mut self) {
        if self.gradients.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "gradient");

        for (id, gradient) in self.gradients.iter() {
            write_gradient(&mut self.xml, &id.to_string(), &gradient.gradient);
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
            // TODO: Respect the xStep/yStep attribute.
            self.xml.write_attribute(
                "width",
                &(pattern.tiling_pattern.bbox.x1 - pattern.tiling_pattern.bbox.x0),
            );
            self.xml.write_attribute(
                "height",
                &(pattern.tiling_pattern.bbox.y1 - pattern.tiling_pattern.bbox.y0),
            );
            self.xml.write_attribute(
                "patternTransform",
                &format!("matrix({})", convert_transform(&transform)),
            );

            pattern.tiling_pattern.interpret(
                self,
                Affine::translate((
                    -pattern.tiling_pattern.bbox.x0,
                    -pattern.tiling_pattern.bbox.y0,
                )),
                false,
            );

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

fn rgba_to_hex(color: [f32; 4]) -> String {
    let r = (color[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let g = (color[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let b = (color[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;

    format!("#{r:02x}{g:02x}{b:02x}")
}

fn gradient_key(shading_key: u128, gradient: &SvgGradient) -> u128 {
    let kind = match &gradient.kind {
        SvgGradientKind::Linear { start, end } => (
            0_u8,
            start.x.to_bits(),
            start.y.to_bits(),
            0_u32,
            end.x.to_bits(),
            end.y.to_bits(),
            0_u32,
            0_u32,
        ),
        SvgGradientKind::Radial {
            start_center,
            start_radius,
            end_center,
            end_radius,
        } => (
            1_u8,
            start_center.x.to_bits(),
            start_center.y.to_bits(),
            start_radius.to_bits(),
            end_center.x.to_bits(),
            end_center.y.to_bits(),
            end_radius.to_bits(),
            0_u32,
        ),
    };

    let transform = gradient.transform.as_coeffs().map(|coeff| coeff.to_bits());
    let stops = gradient
        .stops
        .iter()
        .map(|stop| {
            (
                stop.offset.to_bits(),
                stop.color.map(|component| component.to_bits()),
            )
        })
        .collect::<Vec<_>>();

    hash128(&(shading_key, kind, transform, stops))
}

fn write_gradient(xml: &mut xmlwriter::XmlWriter, id: &str, gradient: &SvgGradient) {
    match &gradient.kind {
        SvgGradientKind::Linear { start, end } => {
            xml.start_element("linearGradient");
            xml.write_attribute("id", id);
            xml.write_attribute("gradientUnits", "userSpaceOnUse");
            xml.write_attribute(
                "gradientTransform",
                &format!(
                    "matrix({})",
                    convert_transform(&(Affine::translate((-0.5, -0.5)) * gradient.transform))
                ),
            );
            xml.write_attribute("x1", &start.x);
            xml.write_attribute("y1", &start.y);
            xml.write_attribute("x2", &end.x);
            xml.write_attribute("y2", &end.y);
        }
        SvgGradientKind::Radial {
            start_center,
            start_radius,
            end_center,
            end_radius,
        } => {
            xml.start_element("radialGradient");
            xml.write_attribute("id", id);
            xml.write_attribute("gradientUnits", "userSpaceOnUse");
            xml.write_attribute(
                "gradientTransform",
                &format!(
                    "matrix({})",
                    convert_transform(&(Affine::translate((-0.5, -0.5)) * gradient.transform))
                ),
            );
            xml.write_attribute("fx", &start_center.x);
            xml.write_attribute("fy", &start_center.y);
            xml.write_attribute("fr", &start_radius);
            xml.write_attribute("cx", &end_center.x);
            xml.write_attribute("cy", &end_center.y);
            xml.write_attribute("r", &end_radius);
        }
    }

    xml.write_attribute("spreadMethod", "pad");

    for stop in &gradient.stops {
        xml.start_element("stop");
        xml.write_attribute("offset", &stop.offset);
        xml.write_attribute("stop-color", &rgba_to_hex(stop.color));
        if stop.color[3] < 1.0 {
            xml.write_attribute("stop-opacity", &stop.color[3]);
        }
        xml.end_element();
    }

    xml.end_element();
}

fn render_shading_texture(
    bbox: Rect,
    shading_pattern: &EncodedShadingPattern,
) -> (DynamicImage, Affine) {
    const SCALE: f32 = 1.0;
    const INV_SCALE: f32 = 1.0 / SCALE;

    let base_width = (bbox.width() as f32).max(1.0);
    let base_height = (bbox.height() as f32).max(1.0);

    let width = (base_width * SCALE).ceil() as u32;
    let height = (base_height * SCALE).ceil() as u32;

    let (x_advance, y_advance) =
        x_y_advances(&(Affine::scale(INV_SCALE as f64) * shading_pattern.base_transform));

    let mut buf = vec![0_u8; width as usize * height as usize * 4];
    let mut start_point = shading_pattern.base_transform
        * Affine::translate((0.5, 0.5))
        * Point::new(bbox.x0, bbox.y0);

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
