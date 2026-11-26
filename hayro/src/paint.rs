use crate::Renderer;
use hayro_interpret::Paint;
use hayro_interpret::encode::{EncodedShadingPattern, EncodedShadingType, texture_dimensions};
use hayro_interpret::gradient::{SvgGradient, SvgGradientKind, SvgGradientStop};
use hayro_interpret::pattern::{Pattern, ShadingPattern};
use hayro_interpret::util::x_y_advances;
use kurbo::{Affine, BezPath, Rect};
use std::sync::Arc;
use vello_cpu::color::{AlphaColor, DynamicColor, Srgb};
use vello_cpu::peniko::{ColorStop, Gradient, ImageQuality, ImageSampler};
use vello_cpu::{Image, ImageSource, PaintType, PixelMetadata, Pixmap, peniko};

const NATIVE_GRADIENT_TOLERANCE: f32 = 0.01;
const STOP_TOLERANCE: f32 = 1.0 / 255.0;

impl Renderer<'_> {
    #[must_use]
    pub(super) fn set_paint(
        &mut self,
        paint: &Paint<'_>,
        path_bbox: impl Fn() -> (Rect, bool),
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
                    let (bounds, has_joins) = path_bbox();
                    let mut bbox = path_transform.transform_rect_bbox(bounds);

                    if is_stroke {
                        bbox = stroke_bounds(
                            bbox,
                            path_transform,
                            self.ctx.stroke(),
                            has_joins && matches!(p.as_ref(), Pattern::Shading(_)),
                        );
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
                        let (paint, transform) = shading_paint(&s, bbox.expand());
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

fn stroke_bounds(bbox: Rect, transform: &Affine, stroke: &kurbo::Stroke, has_joins: bool) -> Rect {
    let (a1, a2) = x_y_advances(transform);
    let pad = a1.length().max(a2.length()) * stroke.width;
    if has_joins && stroke.join == kurbo::Join::Miter {
        let [a, b, c, d, ..] = transform.as_coeffs();
        let radius = stroke.width * 0.5 * stroke.miter_limit.abs();
        bbox.inflate(pad.max(radius * a.hypot(c)), pad.max(radius * b.hypot(d)))
    } else {
        bbox.inflate(pad, pad)
    }
}

fn shading_paint(pattern: &ShadingPattern, mut bbox: Rect) -> (PaintType, Affine) {
    let mut encoded = pattern.encode();
    if let EncodedShadingType::RadialAxial(g) = &mut encoded.shading_type {
        g.cover_placement(bbox, encoded.base_transform);
    }
    if let EncodedShadingType::RadialAxial(g) = &encoded.shading_type {
        let texture_bbox = g.texture_bounds(bbox, &encoded);
        if let Some(mut native) = g.as_raster_gradient(&encoded, STOP_TOLERANCE)
            && raster_precision(&native, bbox)
            && (lut_resolves(&native.stops, NATIVE_GRADIENT_TOLERANCE)
                || texture_bbox.area() > 4096. && retry_plateau(&mut native.stops))
        {
            return native_paint(native);
        }
        bbox = texture_bbox;
    }
    texture_paint(&encoded, bbox)
}

fn raster_precision(gradient: &SvgGradient, bbox: Rect) -> bool {
    let inverse = gradient.transform.inverse();
    if !inverse.is_finite() {
        return false;
    }
    let x = bbox.x0.abs().max(bbox.x1.abs()) + 8.;
    let y = bbox.y0.abs().max(bbox.y1.abs()) + 4.;
    let extent = |a: f64, c: f64, e: f64| a.abs() * x + c.abs() * y + e.abs();
    // Bound the operands before cancellation in Vello's f32 sampler.
    const LIMIT: f64 = 512.;
    let line_to_unit = |p0: kurbo::Point, p1: kurbo::Point| {
        let d = p1 - p0;
        let len2 = d.hypot2();
        if !len2.is_finite() || len2 == 0. {
            return None;
        }
        let ux = d.x / len2;
        let uy = d.y / len2;
        Some(Affine::new([ux, -uy, uy, ux, 0., 0.]) * Affine::translate((-p0.x, -p0.y)))
    };
    match gradient.kind {
        SvgGradientKind::Linear { start, end } => {
            let Some(unit) = line_to_unit(start, end) else {
                return false;
            };
            let eff = unit * inverse;
            let [a, _, c, _, e, _] = eff.as_coeffs();
            eff.is_finite() && extent(a, c, e) < LIMIT
        }
        SvgGradientKind::Radial {
            start_center,
            start_radius,
            end_center,
            end_radius,
        } => {
            let d = (end_center - start_center).hypot();
            if d == 0. {
                let max_r = f64::from(start_radius.max(end_radius));
                let dr = f64::from(end_radius - start_radius).abs();
                if max_r == 0. || dr == 0. {
                    return false;
                }
                let eff = Affine::scale(1. / max_r)
                    * Affine::translate((-end_center.x, -end_center.y))
                    * inverse;
                let [a, b, c, d, e, f] = eff.as_coeffs();
                let scale = max_r / dr;
                let bias = f64::from(start_radius) / dr;
                eff.is_finite()
                    && extent(a, c, e) * scale + bias < LIMIT
                    && extent(b, d, f) * scale + bias < LIMIT
            } else {
                let Some(unit) = line_to_unit(start_center, end_center) else {
                    return false;
                };
                let mut eff = unit * inverse;
                let mut r0 = f64::from(start_radius) / d;
                let mut r1 = f64::from(end_radius) / d;
                let mut focal_x = r0 / (r0 - r1);
                if (focal_x - 1.).abs() < 1e-6 {
                    eff = Affine::scale_non_uniform(-1., 1.) * Affine::translate((-1., 0.)) * eff;
                    std::mem::swap(&mut r0, &mut r1);
                    focal_x = 0.;
                }
                let span = 1. - focal_x;
                let fr1 = r1 / span.abs();
                let denom = fr1 * fr1 - 1.;
                if !denom.is_finite() || denom <= 0. || (focal_x.abs() + 1.) * fr1 / denom >= LIMIT
                {
                    return false;
                }
                let sign = span.signum();
                eff = Affine::scale_non_uniform(sign * fr1 / denom, sign / denom.sqrt())
                    * Affine::translate((-focal_x, 0.))
                    * eff;
                let [a, b, c, d, e, f] = eff.as_coeffs();
                let bias = focal_x.abs();
                eff.is_finite() && extent(a, c, e) + bias < LIMIT && extent(b, d, f) + bias < LIMIT
            }
        }
    }
}

fn native_paint(native: SvgGradient) -> (PaintType, Affine) {
    let mut gradient = match native.kind {
        SvgGradientKind::Linear { start, end } => Gradient::new_linear(start, end),
        SvgGradientKind::Radial {
            start_center,
            start_radius,
            end_center,
            end_radius,
        } => Gradient::new_two_point_radial(start_center, start_radius, end_center, end_radius),
    }
    .with_extend(peniko::Extend::Pad);
    gradient
        .stops
        .extend(native.stops.into_iter().map(|stop| ColorStop {
            offset: stop.offset,
            color: DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new(stop.color)),
        }));

    (PaintType::Gradient(gradient), native.transform)
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

fn retry_plateau(stops: &mut Vec<SvgGradientStop>) -> bool {
    let Some(i) = stops
        .windows(2)
        .position(|w| w[0].color == w[1].color && w[1].offset - w[0].offset > 1. / 4096.)
    else {
        return false;
    };
    let start = stops[i].offset;
    let end = stops[i + 1].offset;
    stops.insert(i + 1, stops[i].clone());
    for size in [2048., 4096.] {
        stops[i + 1].offset = start + 1. / size;
        if stops[i + 1].offset < end && lut_resolves(stops, NATIVE_GRADIENT_TOLERANCE) {
            return true;
        }
    }
    false
}

fn lut_resolves(stops: &[SvgGradientStop], tolerance: f32) -> bool {
    // Match vello_common::encode::{determine_lut_size, GradientLut::new_inner}.
    let mut size = match stops.len() {
        2 => 256,
        3 => 512,
        _ => 1024,
    };
    for pair in stops.windows(2) {
        let width = pair[1].offset - pair[0].offset;
        if width == 0.0 && pair[0].color != pair[1].color {
            return false;
        }
        size = size.max(
            ((1.0 / width).ceil() as usize)
                .min(4096)
                .next_power_of_two(),
        );
    }
    let scale = (size - 1) as f32;
    // The stop approximation spends one byte; rounding both colors can round the difference up.
    let fits = |delta: f32| ((STOP_TOLERANCE + delta) * 255.0).ceil() <= tolerance * 255.0;
    let channel = |stop: &SvgGradientStop, c: usize| {
        if c == 3 {
            stop.color[c]
        } else {
            stop.color[c] * stop.color[3]
        }
    };
    for pair in stops.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let width = b.offset - a.offset;
        if width == 0.0 {
            continue;
        }
        if !(0..4).all(|c| fits((channel(b, c) - channel(a, c)).abs() / (width * scale))) {
            return false;
        }
        if stops.len() == 2 {
            continue;
        }
        // Interior cells follow one slope. Boundary cells can extrapolate another slope.
        for edge in [a.offset, b.offset] {
            let idx = (edge * scale) as usize;
            let segment = stops[1..stops.len() - 1]
                .partition_point(|s| (s.offset * size as f32) as usize <= idx);
            let (left, right) = (&stops[segment], &stops[segment + 1]);
            let t = idx as f32 / scale;
            for c in 0..4 {
                let slope = (channel(right, c) - channel(left, c))
                    / (right.offset - left.offset).max(1.0e-7);
                let sample = slope
                    .mul_add(t, channel(left, c) - left.offset * slope)
                    .clamp(0.0, 1.0);
                for x in [t.max(a.offset), ((idx + 1) as f32 / scale).min(b.offset)] {
                    let expected =
                        channel(a, c) + (channel(b, c) - channel(a, c)) * ((x - a.offset) / width);
                    if !fits((sample - expected).abs()) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_segment_texture_bounds_ignore_miter_limit() {
        use kurbo::Shape;
        let mut path = BezPath::new();
        path.move_to((4000., 4000.));
        path.line_to((4020., 4010.));
        for limit in [10., 100000.] {
            let stroke = kurbo::Stroke::new(2.)
                .with_join(kurbo::Join::Miter)
                .with_miter_limit(limit);
            let bounds = stroke_bounds(
                path.bounding_box(),
                &Affine::IDENTITY,
                &stroke,
                crate::path::path_has_joins(&path),
            );
            assert_eq!(
                bounds.intersect(Rect::new(0., 0., 8192., 8192.)).size(),
                kurbo::Size::new(24., 14.)
            );
        }
    }

    #[test]
    fn plateau_enlarges_lut_for_adjacent_ramp() {
        let mut stops = [(0., 0.), (0.2, 0.), (0.4, 1.), (0.5, 1.), (1., 0.)]
            .map(|(offset, v)| SvgGradientStop {
                offset,
                color: [v, v, v, 1.],
            })
            .to_vec();
        assert!(!lut_resolves(&stops, NATIVE_GRADIENT_TOLERANCE));
        assert!(retry_plateau(&mut stops));
        assert!(lut_resolves(&stops, NATIVE_GRADIENT_TOLERANCE));
    }

    #[test]
    fn lut_rejects_stitched_band() {
        let stops = [
            (0.0, 0.0),
            (0.025, 0.0),
            (0.025, 1.0),
            (0.075, 1.0),
            (0.075, 0.0),
            (1.0, 0.0),
        ]
        .map(|(offset, gray)| SvgGradientStop {
            offset,
            color: [gray, gray, gray, 1.0],
        });
        assert!(!lut_resolves(&stops, 0.01));
    }

    #[test]
    fn raster_precision_rejects_ill_conditioned_radial_gradients() {
        let bbox = Rect::new(0., 0., 16., 16.);
        let focal = SvgGradient {
            kind: SvgGradientKind::Radial {
                start_center: kurbo::Point::ORIGIN,
                start_radius: 100.,
                end_center: kurbo::Point::new(1., 0.),
                end_radius: 101.00025,
            },
            transform: Affine::IDENTITY,
            stops: Vec::new(),
        };
        assert!(!raster_precision(&focal, bbox));
        // Even zoomed into the left rim where eff extents shrink, f32 focal coefficients lose precision.
        let zoomed_focal = SvgGradient {
            transform: Affine::translate((100., 0.)) * Affine::scale(64.),
            ..focal
        };
        assert!(!raster_precision(&zoomed_focal, bbox));
        let near_tangent = SvgGradient {
            kind: SvgGradientKind::Radial {
                start_center: kurbo::Point::ORIGIN,
                start_radius: 128.03,
                end_center: kurbo::Point::new(1., 0.),
                end_radius: 129.031,
            },
            transform: Affine::translate((128.03 * 64., 0.)) * Affine::scale(64.),
            stops: Vec::new(),
        };
        assert!(!raster_precision(&near_tangent, bbox));

        let concentric = SvgGradient {
            kind: SvgGradientKind::Radial {
                start_center: kurbo::Point::ORIGIN,
                start_radius: 100.,
                end_center: kurbo::Point::ORIGIN,
                end_radius: 100.05,
            },
            transform: Affine::IDENTITY,
            stops: Vec::new(),
        };
        assert!(!raster_precision(&concentric, bbox));
    }
}
