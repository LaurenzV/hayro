//! SVG-like gradient conversion for encoded shadings.
//!
//! PDF shadings are very expensive to evaluate. Because of this, at least for
//! radial-axial shadings it's useful if we can instead approximate them by
//! SVG-like sRGB gradients.

use crate::color::AlphaColor;
use crate::encode::{EncodedRadialAxialShading, EncodedShadingPattern, RadialAxialParams};
use crate::function::StitchingBounds;
use kurbo::{Affine, Point, Rect};
use smallvec::smallvec;

const MAX_GRADIENT_SUBDIVISION_DEPTH: u8 = 10;

/// An SVG-like gradient.
pub struct SvgGradient {
    /// The kind of the gradient.
    pub kind: SvgGradientKind,
    /// The transform from encoded shading coordinates into the renderer's coordinate space.
    pub transform: Affine,
    /// The stops of the gradient in the sRGB color space.
    pub stops: Vec<SvgGradientStop>,
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
pub struct SvgGradientStop {
    /// Normalized offset.
    pub offset: f32,
    /// RGBA color components in sRGB.
    pub color: [f32; 4],
}

impl EncodedRadialAxialShading {
    /// Convert an axial/radial PDF shading into an SVG-like gradient approximation.
    pub fn as_svg_gradient(
        &self,
        pattern: &EncodedShadingPattern,
        path_bbox: Rect,
        tolerance: f32,
    ) -> Option<SvgGradient> {
        // A couple of cases cannot be losslessly represented by an SVG gradient.
        match self.params {
            RadialAxialParams::Axial => {
                if !self.axial_extend_covers_bbox(pattern, path_bbox) {
                    return None;
                }
            }
            RadialAxialParams::Radial { .. } => {
                if !self.extend.iter().all(|e| *e) || pattern.background_color.components()[3] > 0.0
                {
                    return None;
                }
            }
        }

        Some(SvgGradient {
            kind: self.native_gradient_kind(),
            transform: pattern.base_transform.inverse(),
            stops: approximate_gradient_stops(
                |t| self.sample_t(pattern, t),
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
        let (t0, t1) = (self.domain[0], self.domain[1]);
        let t = t0 + (t1 - t0) * t;
        let Some(out) = self.function.eval(&smallvec![t]) else {
            return [0.0, 0.0, 0.0, 0.0];
        };
        let mut components = pattern.color_space.to_rgba(&out, 1.0, false).components();
        components[3] *= pattern.opacity;

        if let Some(tf) = &pattern.transfer_function {
            return tf.apply(&AlphaColor::new(components)).components();
        }

        components
    }

    fn axial_extend_covers_bbox(&self, pattern: &EncodedShadingPattern, path_bbox: Rect) -> bool {
        let path_bbox = pattern.base_transform.transform_rect_bbox(path_bbox);

        let min_x = path_bbox.min_x();
        let max_x = path_bbox.max_x();

        (self.extend[0] || min_x >= 0.0) && (self.extend[1] || max_x <= 1.0)
    }

    fn normalized_stitching_bounds(&self) -> StitchingBounds {
        let [t0, t1] = self.domain;
        let dt = t1 - t0;
        if dt.abs() <= f32::EPSILON {
            return StitchingBounds::new();
        }

        let mut bounds = self
            .function
            .stitching_bounds()
            .into_iter()
            .map(|bound| (bound - t0) / dt)
            .filter(|bound| *bound > 0.0 && *bound < 1.0)
            .collect::<StitchingBounds>();

        if dt < 0.0 {
            bounds.reverse();
        }

        bounds
    }
}

fn approximate_gradient_stops(
    mut sample: impl FnMut(f32) -> [f32; 4],
    breakpoints: &[f32],
    tolerance: f32,
) -> Vec<SvgGradientStop> {
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
struct GradientSample {
    offset: f32,
    color: [f32; 4],
}

fn approximate_gradient_interval(
    sample: &mut impl FnMut(f32) -> [f32; 4],
    stops: &mut Vec<SvgGradientStop>,
    start: GradientSample,
    end: GradientSample,
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

fn lerp_color(c0: [f32; 4], c1: [f32; 4], t: f32) -> [f32; 4] {
    [
        c0[0] + (c1[0] - c0[0]) * t,
        c0[1] + (c1[1] - c0[1]) * t,
        c0[2] + (c1[2] - c0[2]) * t,
        c0[3] + (c1[3] - c0[3]) * t,
    ]
}

fn color_error(c0: [f32; 4], c1: [f32; 4]) -> f32 {
    c0.iter()
        .zip(c1)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}
