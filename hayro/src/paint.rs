use crate::Renderer;
use hayro_interpret::Paint;
use hayro_interpret::encode::{EncodedShadingType, texture_dimensions};
use hayro_interpret::gradient::SvgGradientKind;
use hayro_interpret::pattern::{Pattern, TilingPattern};
use hayro_interpret::util::x_y_advances;
use kurbo::{Affine, BezPath, Rect, Shape};
use std::sync::Arc;
use vello_cpu::color::{AlphaColor, DynamicColor, Srgb};
use vello_cpu::peniko::{ColorStop, Gradient, ImageQuality, ImageSampler};
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

                match *p {
                    Pattern::Shading(s) => {
                        const NATIVE_GRADIENT_TOLERANCE: f32 = 0.01;

                        clip_path = s.shading.clip_path.clone();
                        let encoded = s.encode();
                        let bbox = self.device_bbox(path_transform, path_bbox(), is_stroke);

                        if let EncodedShadingType::RadialAxial(gradient) = &encoded.shading_type
                            && let Some(native) =
                                gradient.as_svg_gradient(&encoded, bbox, NATIVE_GRADIENT_TOLERANCE)
                        {
                            paint_transform = path_transform.inverse()
                                * Affine::translate((-0.5, -0.5))
                                * native.transform;

                            let stops = native
                                .stops
                                .iter()
                                .map(|stop| ColorStop {
                                    offset: stop.offset,
                                    color: DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new(
                                        stop.color,
                                    )),
                                })
                                .collect::<Vec<_>>();

                            let gradient = match native.kind {
                                SvgGradientKind::Linear { start, end } => {
                                    Gradient::new_linear(start, end)
                                }
                                SvgGradientKind::Radial {
                                    start_center,
                                    start_radius,
                                    end_center,
                                    end_radius,
                                } => Gradient::new_two_point_radial(
                                    start_center,
                                    start_radius,
                                    end_center,
                                    end_radius,
                                ),
                            }
                            .with_extend(peniko::Extend::Pad)
                            .with_stops(stops.as_slice());

                            PaintType::Gradient(gradient)
                        } else {
                            let (width, height) = texture_dimensions(bbox, 1.0);
                            let mut image = Vec::with_capacity(width as usize * height as usize);
                            let mut may_have_transparency = false;
                            let (width, height, transform) =
                                encoded.sample_texture(bbox, 1.0, |sample| {
                                    let pixel =
                                        AlphaColor::<Srgb>::new(sample).premultiply().to_rgba8();
                                    may_have_transparency |= pixel.a != 255;
                                    image.push(pixel);
                                });
                            paint_transform = path_transform.inverse() * transform;

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
                                    x_extend: peniko::Extend::Repeat,
                                    y_extend: peniko::Extend::Repeat,
                                    quality: ImageQuality::Medium,
                                    alpha: 1.0,
                                },
                            };

                            PaintType::Image(image)
                        }
                    }
                    Pattern::Tiling(t) => {
                        const MAX_PIXMAP_SIZE: f32 = 3000.0;
                        // TODO: Raise this limit and perform downsampling if reached
                        // (see pdftc_100k_0138.pdf).
                        const MIN_PIXMAP_SIZE: f32 = 1.0;

                        let bbox = t.bbox;
                        let max_x_scale = MAX_PIXMAP_SIZE / bbox.width() as f32;
                        let min_x_scale = MIN_PIXMAP_SIZE / bbox.width() as f32;
                        let max_y_scale = MAX_PIXMAP_SIZE / bbox.height() as f32;
                        let min_y_scale = MIN_PIXMAP_SIZE / bbox.height() as f32;

                        let (mut xs, mut ys) = {
                            let (x, y) = x_y_advances(&(t.matrix));
                            (x.length() as f32, y.length() as f32)
                        };
                        xs = xs.max(min_x_scale).min(max_x_scale);
                        ys = ys.max(min_y_scale).min(max_y_scale);

                        // The pixmap holds one whole step. If the step is too large for
                        // that and the painted area touches only one tile, render just
                        // the bbox and place it once.
                        let single_tile = if (xs * t.x_step).abs() > MAX_PIXMAP_SIZE
                            || (ys * t.y_step).abs() > MAX_PIXMAP_SIZE
                        {
                            let area = self.device_bbox(path_transform, path_bbox(), is_stroke);
                            visible_tile(&t, area)
                        } else {
                            None
                        };

                        let (pix, extend) = if let Some((x_offset, y_offset)) = single_tile {
                            // Keep a transparent border so that padding adds nothing.
                            let pix_width = (bbox.width() as f32 * xs).ceil() as u16 + 2;
                            let pix_height = (bbox.height() as f32 * ys).ceil() as u16 + 2;
                            let initial_transform = Affine::translate((1.0, 1.0))
                                * Affine::scale_non_uniform(xs as f64, ys as f64)
                                * Affine::translate((-bbox.x0, -bbox.y0));
                            let pix = self.render_tile(
                                &t,
                                pix_width,
                                pix_height,
                                initial_transform,
                                is_stroke,
                            );

                            paint_transform = path_transform.inverse()
                                * t.matrix
                                * Affine::translate((x_offset, y_offset))
                                * initial_transform.inverse();

                            (pix, peniko::Extend::Pad)
                        } else {
                            // The pixmap is sized by the step, so limit the scale by
                            // the step as well.
                            xs = xs.min(MAX_PIXMAP_SIZE / t.x_step.abs());
                            ys = ys.min(MAX_PIXMAP_SIZE / t.y_step.abs());

                            let x_step = xs * t.x_step;
                            let y_step = ys * t.y_step;

                            let pix_width = x_step.abs().round().max(1.0) as u16;
                            let pix_height = y_step.abs().round().max(1.0) as u16;

                            let xs = pix_width as f32 / t.x_step.abs();
                            let ys = pix_height as f32 / t.y_step.abs();

                            let scaled_width = bbox.width() as f32 * xs;
                            let scaled_height = bbox.height() as f32 * ys;

                            let mut initial_transform =
                                Affine::scale_non_uniform(xs as f64, ys as f64)
                                    * Affine::translate((-bbox.x0, -bbox.y0));
                            let pix = self.render_tile(
                                &t,
                                pix_width,
                                pix_height,
                                initial_transform,
                                is_stroke,
                            );

                            // TODO: Fix these
                            if x_step < 0.0 {
                                initial_transform *=
                                    Affine::new([-1.0, 0.0, 0.0, 1.0, scaled_width as f64, 0.0]);
                            }

                            if y_step < 0.0 {
                                initial_transform *=
                                    Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, scaled_height as f64]);
                            }

                            paint_transform =
                                path_transform.inverse() * t.matrix * initial_transform.inverse();

                            (pix, peniko::Extend::Repeat)
                        };

                        let image = Image {
                            image: ImageSource::Pixmap(Arc::new(pix)),
                            sampler: ImageSampler {
                                x_extend: extend,
                                y_extend: extend,
                                quality: ImageQuality::Medium,
                                alpha: 1.0,
                            },
                        };

                        PaintType::Image(image)
                    }
                }
            }
        };

        self.ctx.set_paint_transform(paint_transform);
        self.ctx.set_paint(paint);

        clip_path
    }

    fn device_bbox(&self, path_transform: &Affine, path_bbox: Rect, is_stroke: bool) -> Rect {
        let mut bbox = (*path_transform * path_bbox.to_path(0.0)).bounding_box();

        if is_stroke {
            // Try to account for stroke in bbox.
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
    }

    fn render_tile(
        &self,
        t: &TilingPattern<'_>,
        width: u16,
        height: u16,
        initial_transform: Affine,
        is_stroke: bool,
    ) -> Pixmap {
        let mut ctx = self.child_context(width, height);
        let mut renderer = Renderer::new(&mut ctx, self.global);
        renderer.inside_pattern = true;
        t.interpret(&mut renderer, initial_transform, is_stroke);
        let mut pix = Pixmap::new(width, height);
        renderer.ctx.flush();
        let mut resources = vello_cpu::Resources::default();
        renderer.ctx.render(&mut pix, &mut resources);

        pix
    }
}

/// Return the offset of the only tile that touches the device area, if there is at most one.
fn visible_tile(t: &TilingPattern<'_>, area: Rect) -> Option<(f64, f64)> {
    let area = (t.matrix.inverse() * area.to_path(0.1)).bounding_box();

    let offset = |min: f64, max: f64, start: f64, end: f64, step: f32| {
        let step = step.abs() as f64;
        let first = ((min - end) / step).ceil();
        let last = ((max - start) / step).floor();

        (first >= last).then_some(first * step)
    };

    Some((
        offset(area.x0, area.x1, t.bbox.x0, t.bbox.x1, t.x_step)?,
        offset(area.y0, area.y1, t.bbox.y0, t.bbox.y1, t.y_step)?,
    ))
}
