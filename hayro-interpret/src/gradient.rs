//! SVG-like gradient conversion for encoded shadings.
//!
//! PDF shadings are very expensive to evaluate. Because of this, at least for
//! radial-axial shadings it's useful if we can instead approximate them by
//! SVG-like sRGB gradients.

use crate::color::{color_error, lerp_color};
use crate::encode::{EncodedRadialAxialShading, EncodedShadingPattern, RadialAxialParams};
use crate::shading::ShadingFunction;
use kurbo::{Affine, Point};

const MAX_GRADIENT_SUBDIVISION_DEPTH: u8 = 10;
const RADIAL_EPSILON: f64 = 1.0e-6;
const PADDING: f64 = 0.01;
const LUT_TOLERANCE: f32 = 0.01;
const MIN_LUT_SIZE: usize = 256;
const MAX_LUT_SIZE: usize = 4096;

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
        tolerance: f32,
    ) -> Option<SvgGradient> {
        let background = pattern.background_color.components();
        let background = (background[3] > 0.0).then(|| pattern.finish(background));
        let range = self.padded_range(background.is_some())?;

        Some(SvgGradient {
            kind: self.gradient_kind(range),
            transform: pattern.base_transform.inverse(),
            stops: pad_stops(
                approximate_gradient_stops(
                    |t, colors| sample_function(&self.function, self.domain, pattern, t, colors),
                    &normalized_stitching_bounds(&self.function, self.domain),
                    tolerance,
                ),
                range,
                background,
            ),
        })
    }

    // SVG gradients pad with their end stops, so a non-extended end gets a short stretch of background.
    fn padded_range(&self, has_background: bool) -> Option<[f64; 2]> {
        let mut padding = self.extend.map(|extend| if extend { 0.0 } else { PADDING });

        if let RadialAxialParams::Radial { p1, r } = self.params {
            let growth = r.y - r.x;
            let nested = growth.abs() - p1.to_vec2().hypot() > RADIAL_EPSILON;

            // Unless nested, circles past the end outrank earlier ones and some points need the background.
            if r.x < 0.0 || r.y < 0.0 || !nested && (has_background || !self.extend[1]) {
                return None;
            }

            // The padded circles must not have a negative radius.
            padding[0] = padding[0].min(r.x / growth.max(0.0));
            padding[1] = padding[1].min(r.y / (-growth).max(0.0));
        }

        Some([-padding[0], 1.0 + padding[1]])
    }

    fn gradient_kind(&self, [t0, t1]: [f64; 2]) -> SvgGradientKind {
        match &self.params {
            RadialAxialParams::Axial => SvgGradientKind::Linear {
                start: Point::new(t0, 0.0),
                end: Point::new(t1, 0.0),
            },
            RadialAxialParams::Radial { p1, r } => SvgGradientKind::Radial {
                start_center: (p1.to_vec2() * t0).to_point(),
                start_radius: (r.x + (r.y - r.x) * t0) as f32,
                end_center: (p1.to_vec2() * t1).to_point(),
                end_radius: (r.x + (r.y - r.x) * t1) as f32,
            },
        }
    }
}

/// A lookup table of `function`'s colors over `range`, indexed by the normalized position in it.
pub(crate) fn function_lut(
    function: &ShadingFunction,
    range: [f32; 2],
    pattern: &EncodedShadingPattern,
) -> ColorLut {
    ColorLut::new(&approximate_gradient_stops(
        |t, colors| sample_function(function, range, pattern, t, colors),
        &normalized_stitching_bounds(function, range),
        LUT_TOLERANCE,
    ))
}

fn sample_function(
    function: &ShadingFunction,
    [t0, t1]: [f32; 2],
    pattern: &EncodedShadingPattern,
    t: &[f32],
    colors: &mut [[f32; 4]],
) {
    const CHUNK: usize = 16;

    let mut inputs = [0.0; CHUNK];

    for (t, colors) in t.chunks(CHUNK).zip(colors.chunks_mut(CHUNK)) {
        for (input, t) in inputs.iter_mut().zip(t) {
            *input = t0 + (t1 - t0) * t;
        }

        pattern.eval_many(function, &inputs[..t.len()], colors);
    }
}

fn normalized_stitching_bounds(function: &ShadingFunction, [t0, t1]: [f32; 2]) -> Vec<f32> {
    let dt = t1 - t0;
    if dt.abs() <= f32::EPSILON {
        return vec![];
    }

    let mut bounds = function
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

pub(crate) struct ColorLut {
    colors: Vec<[f32; 4]>,
}

impl ColorLut {
    fn new(stops: &[SvgGradientStop]) -> Self {
        let min_gap = stops
            .windows(2)
            .map(|w| w[1].offset - w[0].offset)
            .filter(|gap| *gap > 0.0)
            .fold(1.0, f32::min);
        let size = ((1.0 / min_gap).ceil() as usize)
            .next_power_of_two()
            .clamp(MIN_LUT_SIZE, MAX_LUT_SIZE);

        let mut segment = 0;
        let colors = (0..size)
            .map(|i| {
                let t = i as f32 / (size - 1) as f32;

                while segment + 2 < stops.len() && stops[segment + 1].offset < t {
                    segment += 1;
                }

                let (start, end) = (&stops[segment], &stops[(segment + 1).min(stops.len() - 1)]);
                let width = end.offset - start.offset;
                let local = if width > 0.0 {
                    ((t - start.offset) / width).clamp(0.0, 1.0)
                } else {
                    1.0
                };

                lerp_color(start.color, end.color, local)
            })
            .collect();

        Self { colors }
    }

    #[inline]
    pub(crate) fn get(&self, t: f32) -> [f32; 4] {
        let last = self.colors.len() - 1;
        self.colors[((t * last as f32 + 0.5) as usize).min(last)]
    }
}

fn pad_stops(
    mut stops: Vec<SvgGradientStop>,
    [t0, t1]: [f64; 2],
    background: Option<[f32; 4]>,
) -> Vec<SvgGradientStop> {
    let (t0, t1) = (t0 as f32, t1 as f32);
    let padding = |offset| SvgGradientStop {
        offset,
        color: background.unwrap_or([0.0; 4]),
    };

    for stop in &mut stops {
        stop.offset = (stop.offset - t0) / (t1 - t0);
    }

    if t0 < 0.0 {
        let edge = stops[0].offset;
        stops.splice(0..0, [padding(0.0), padding(edge)]);
    }

    if t1 > 1.0 {
        let edge = stops[stops.len() - 1].offset;
        stops.extend([padding(edge), padding(1.0)]);
    }

    stops
}

fn approximate_gradient_stops(
    mut sample: impl FnMut(&[f32], &mut [[f32; 4]]),
    breakpoints: &[f32],
    tolerance: f32,
) -> Vec<SvgGradientStop> {
    let mut interval_bounds = Vec::with_capacity(breakpoints.len() + 2);
    interval_bounds.push(0.0);
    interval_bounds.extend(breakpoints.iter().copied());
    interval_bounds.push(1.0);

    let mut bound_colors = vec![[0.0; 4]; interval_bounds.len()];
    sample(&interval_bounds, &mut bound_colors);

    let mut stops = Vec::new();
    for (offsets, colors) in interval_bounds.windows(2).zip(bound_colors.windows(2)) {
        let [start, end] = [0, 1].map(|i| GradientSample {
            offset: offsets[i],
            color: colors[i],
        });

        stops.push(SvgGradientStop {
            offset: start.offset,
            color: start.color,
        });

        approximate_gradient_interval(&mut sample, &mut stops, start, end, tolerance.max(0.0), 0);
    }

    stops
}

#[derive(Clone, Copy)]
struct GradientSample {
    offset: f32,
    color: [f32; 4],
}

fn approximate_gradient_interval(
    sample: &mut impl FnMut(&[f32], &mut [[f32; 4]]),
    stops: &mut Vec<SvgGradientStop>,
    start: GradientSample,
    end: GradientSample,
    tolerance: f32,
    depth: u8,
) {
    let offset_delta = end.offset - start.offset;
    let offsets = [0.25, 0.5, 0.75].map(|f| start.offset + offset_delta * f);
    let mid_offset = offsets[1];

    let mut colors = [[0.0; 4]; 3];
    sample(&offsets, &mut colors);
    let [quarter_color, mid_color, three_quarter_color] = colors;

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
