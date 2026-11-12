//! SVG-like gradient conversion for encoded shadings.
//!
//! PDF shadings are very expensive to evaluate. Because of this, at least for
//! radial-axial shadings it's useful if we can instead approximate them by
//! SVG-like sRGB gradients, or CMYK gradients for CMYK output.

use crate::color::{AlphaColor, CmykProfile, srgb_to_cmyk};
use crate::encode::{EncodedRadialAxialShading, EncodedShadingPattern, RadialAxialParams};
use crate::function::MAX_COMPONENTS;
use kurbo::{Affine, Circle, Point, Rect, Shape};

const MAX_GRADIENT_SUBDIVISION_DEPTH: u8 = 10;
const RADIAL_EPSILON: f64 = 1.0e-6;

/// An SVG-like gradient.
///
/// The colors of its stops are RGBA in sRGB, or cyan, magenta, yellow, black and alpha for
/// [`EncodedRadialAxialShading::as_svg_gradient_cmyk`].
pub struct SvgGradient<C = [f32; 4]> {
    /// The kind of the gradient.
    pub kind: SvgGradientKind,
    /// The transform from encoded shading coordinates into the renderer's coordinate space.
    pub transform: Affine,
    /// The stops of the gradient.
    pub stops: Vec<SvgGradientStop<C>>,
}

/// The kind of the SVG gradient.
pub enum SvgGradientKind {
    /// Linear gradient.
    Linear {
        /// The normalized start point of the gradient line.
        start: Point,
        /// The normalized end point of the gradient line.
        end: Point,
    },
    /// Radial gradient.
    Radial {
        /// The center point of the start circle.
        start_center: Point,
        /// The radius of the start circle.
        start_radius: f32,
        /// The center point of the end circle.
        end_center: Point,
        /// The radius of the end circle.
        end_radius: f32,
    },
}

/// A color stop for an SVG-like gradient.
pub struct SvgGradientStop<C = [f32; 4]> {
    /// Normalized offset.
    pub offset: f32,
    /// The color components, see [`SvgGradient`].
    pub color: C,
}

impl EncodedRadialAxialShading {
    /// Convert an axial/radial PDF shading into an SVG-like gradient approximation.
    pub fn as_svg_gradient(
        &self,
        pattern: &EncodedShadingPattern,
        path_bbox: Rect,
        tolerance: f32,
    ) -> Option<SvgGradient> {
        self.as_gradient(pattern, path_bbox, tolerance, |t| self.sample_t(pattern, t))
    }

    /// Like [`as_svg_gradient`](Self::as_svg_gradient), with the colors of the stops as
    /// cyan, magenta, yellow, black and alpha, converted as by
    /// [`EncodedShadingPattern::sample_cmyk`].
    pub fn as_svg_gradient_cmyk(
        &self,
        pattern: &EncodedShadingPattern,
        path_bbox: Rect,
        tolerance: f32,
        profile: Option<&CmykProfile>,
    ) -> Option<SvgGradient<[f32; 5]>> {
        self.as_gradient(pattern, path_bbox, tolerance, |t| {
            self.sample_t_cmyk(pattern, t, profile)
        })
    }

    fn as_gradient<const N: usize>(
        &self,
        pattern: &EncodedShadingPattern,
        path_bbox: Rect,
        tolerance: f32,
        sample: impl FnMut(f32) -> [f32; N],
    ) -> Option<SvgGradient<[f32; N]>> {
        // A couple of cases cannot be losslessly represented by an SVG gradient.
        match self.params {
            RadialAxialParams::Axial => {
                if !self.axial_extend_covers_bbox(pattern, path_bbox) {
                    return None;
                }
            }
            RadialAxialParams::Radial { .. } => {
                if !self.radial_extend_covers_bbox(pattern, path_bbox)
                    || pattern.background_color.components()[3] > 0.0
                {
                    return None;
                }
            }
        }

        Some(SvgGradient {
            kind: self.native_gradient_kind(),
            transform: pattern.base_transform.inverse(),
            stops: approximate_gradient_stops(
                sample,
                &self.normalized_stitching_bounds(),
                tolerance,
            ),
        })
    }

    fn native_gradient_kind(&self) -> SvgGradientKind {
        match &self.params {
            RadialAxialParams::Axial => SvgGradientKind::Linear {
                start: Point::ZERO,
                end: Point::new(1.0, 0.0),
            },
            RadialAxialParams::Radial { p1, r } => SvgGradientKind::Radial {
                start_center: Point::ZERO,
                start_radius: r.x as f32,
                end_center: *p1,
                end_radius: r.y as f32,
            },
        }
    }

    fn sample_t(&self, pattern: &EncodedShadingPattern, t: f32) -> [f32; 4] {
        let mut components = self.eval_t(t, [0.0; 4], |out| {
            pattern.color_space.to_rgba(out, 1.0).components()
        });
        components[3] *= pattern.opacity;

        if let Some(tf) = &pattern.transfer_function {
            return tf.apply(&AlphaColor::new(components)).components();
        }

        components
    }

    fn sample_t_cmyk(
        &self,
        pattern: &EncodedShadingPattern,
        t: f32,
        profile: Option<&CmykProfile>,
    ) -> [f32; 5] {
        // As in `EncodedShadingPattern::sample_cmyk`, transfer functions apply to sRGB.
        if pattern.transfer_function.is_some() {
            let [r, g, b, a] = self.sample_t(pattern, t);
            let [c, m, y, k] = srgb_to_cmyk([r, g, b], profile);
            return [c, m, y, k, a];
        }

        let mut components = self.eval_t(t, [0.0; 5], |out| {
            pattern.color_space.to_cmyk(out, 1.0, profile)
        });
        components[4] *= pattern.opacity;

        components
    }

    fn eval_t<T>(&self, t: f32, invalid: T, color: impl FnOnce(&[f32]) -> T) -> T {
        let (t0, t1) = (self.domain[0], self.domain[1]);
        let t = t0 + (t1 - t0) * t;
        let mut out = [0.0; MAX_COMPONENTS];
        let out = &mut out[..self.function.num_outputs()];
        if self.function.eval(&[t], out).is_none() {
            return invalid;
        }

        color(out)
    }

    fn axial_extend_covers_bbox(&self, pattern: &EncodedShadingPattern, path_bbox: Rect) -> bool {
        let path_bbox = pattern.base_transform.transform_rect_bbox(path_bbox);

        let min_x = path_bbox.min_x();
        let max_x = path_bbox.max_x();

        (self.extend[0] || min_x >= 0.0) && (self.extend[1] || max_x <= 1.0)
    }

    fn radial_extend_covers_bbox(&self, pattern: &EncodedShadingPattern, path_bbox: Rect) -> bool {
        let RadialAxialParams::Radial { p1, r } = self.params else {
            return false;
        };

        let path_bbox = pattern.base_transform.transform_rect_bbox(path_bbox);

        if p1.x.abs() <= RADIAL_EPSILON && p1.y.abs() <= RADIAL_EPSILON {
            return self.same_center_radial_extend_covers_bbox(path_bbox, r.x, r.y);
        }

        let start_extend_ok = self.extend[0] || r.x.abs() <= RADIAL_EPSILON;
        let end_extend_ok = self.extend[1] || circle_covers_bbox(Circle::new(p1, r.y), path_bbox);

        start_extend_ok && end_extend_ok
    }

    fn same_center_radial_extend_covers_bbox(&self, path_bbox: Rect, r0: f64, r1: f64) -> bool {
        let mut inner_circle = Circle::new(Point::ZERO, r0);
        let mut outer_circle = Circle::new(Point::ZERO, r1);
        let mut inner_extend = self.extend[0];
        let mut outer_extend = self.extend[1];

        if inner_circle.radius > outer_circle.radius {
            std::mem::swap(&mut inner_circle, &mut outer_circle);
            std::mem::swap(&mut inner_extend, &mut outer_extend);
        }

        let inner_extend_ok = inner_extend
            || inner_circle.radius <= RADIAL_EPSILON
            || circle_outside_bbox(inner_circle, path_bbox);
        let outer_extend_ok = outer_extend || circle_covers_bbox(outer_circle, path_bbox);

        inner_extend_ok && outer_extend_ok
    }

    fn normalized_stitching_bounds(&self) -> Vec<f32> {
        let [t0, t1] = self.domain;
        let dt = t1 - t0;
        if dt.abs() <= f32::EPSILON {
            return vec![];
        }

        let mut bounds = self
            .function
            .stitching_bounds()
            .into_iter()
            .map(|bound| (bound - t0) / dt)
            .filter(|bound| *bound > 0.0 && *bound < 1.0)
            .collect::<Vec<_>>();

        if dt < 0.0 {
            bounds.reverse();
        }

        bounds
    }
}

fn circle_covers_bbox(circle: Circle, bbox: Rect) -> bool {
    if circle.radius < 0.0 {
        return false;
    }

    let radius_squared = circle.radius * circle.radius;
    [
        Point::new(bbox.x0, bbox.y0),
        Point::new(bbox.x1, bbox.y0),
        Point::new(bbox.x0, bbox.y1),
        Point::new(bbox.x1, bbox.y1),
    ]
    .into_iter()
    .all(|point| {
        let delta = point - circle.center;
        delta.hypot2() <= radius_squared
    })
}

fn circle_outside_bbox(circle: Circle, bbox: Rect) -> bool {
    if circle.radius <= 0.0 {
        return true;
    }

    let closest = Point::new(
        circle.center.x.clamp(bbox.x0, bbox.x1),
        circle.center.y.clamp(bbox.y0, bbox.y1),
    );

    !circle.contains(closest)
}

fn approximate_gradient_stops<const N: usize>(
    mut sample: impl FnMut(f32) -> [f32; N],
    breakpoints: &[f32],
    tolerance: f32,
) -> Vec<SvgGradientStop<[f32; N]>> {
    let mut interval_bounds = Vec::with_capacity(breakpoints.len() + 2);
    interval_bounds.push(0.0);
    interval_bounds.extend(breakpoints.iter().copied());
    interval_bounds.push(1.0);

    let mut stops = Vec::new();
    for interval in interval_bounds.windows(2) {
        let start_offset = interval[0];
        let end_offset = interval[1];

        let start_color = sample(start_offset);
        stops.push(SvgGradientStop {
            offset: start_offset,
            color: start_color,
        });

        let end_color = sample(end_offset);

        approximate_gradient_interval(
            &mut sample,
            &mut stops,
            GradientSample {
                offset: start_offset,
                color: start_color,
            },
            GradientSample {
                offset: end_offset,
                color: end_color,
            },
            tolerance.max(0.0),
            0,
        );
    }

    stops
}

#[derive(Clone, Copy)]
struct GradientSample<const N: usize> {
    offset: f32,
    color: [f32; N],
}

fn approximate_gradient_interval<const N: usize>(
    sample: &mut impl FnMut(f32) -> [f32; N],
    stops: &mut Vec<SvgGradientStop<[f32; N]>>,
    start: GradientSample<N>,
    end: GradientSample<N>,
    tolerance: f32,
    depth: u8,
) {
    let offset_delta = end.offset - start.offset;
    let quarter_offset = start.offset + offset_delta * 0.25;
    let mid_offset = start.offset + offset_delta * 0.5;
    let three_quarter_offset = start.offset + offset_delta * 0.75;

    let quarter_color = sample(quarter_offset);
    let mid_color = sample(mid_offset);
    let three_quarter_color = sample(three_quarter_offset);

    if color_error(quarter_color, lerp_color(start.color, end.color, 0.25)) <= tolerance
        && color_error(mid_color, lerp_color(start.color, end.color, 0.5)) <= tolerance
        && color_error(
            three_quarter_color,
            lerp_color(start.color, end.color, 0.75),
        ) <= tolerance
        || depth == MAX_GRADIENT_SUBDIVISION_DEPTH
    {
        stops.push(SvgGradientStop {
            offset: end.offset,
            color: end.color,
        });

        return;
    }

    let mid = GradientSample {
        offset: mid_offset,
        color: mid_color,
    };

    approximate_gradient_interval(sample, stops, start, mid, tolerance, depth + 1);
    approximate_gradient_interval(sample, stops, mid, end, tolerance, depth + 1);
}

fn lerp_color<const N: usize>(c0: [f32; N], c1: [f32; N], t: f32) -> [f32; N] {
    core::array::from_fn(|index| c0[index] + (c1[index] - c0[index]) * t)
}

fn color_error<const N: usize>(c0: [f32; N], c1: [f32; N]) -> f32 {
    c0.iter()
        .zip(c1)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}
