//! SVG-like gradient approximations of encoded shadings.

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
                approximate_gradient_stops(&self.function, self.domain, pattern, tolerance),
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

pub(crate) fn function_lut(
    function: &ShadingFunction,
    range: [f32; 2],
    pattern: &EncodedShadingPattern,
) -> ColorLut {
    ColorLut::new(&approximate_gradient_stops(
        function,
        range,
        pattern,
        LUT_TOLERANCE,
    ))
}

pub(crate) struct ColorLut {
    colors: Vec<[f32; 4]>,
}

impl ColorLut {
    #[inline]
    pub(crate) fn get(&self, t: f32) -> [f32; 4] {
        let last = self.colors.len() - 1;
        self.colors[((t * last as f32 + 0.5) as usize).min(last)]
    }

    fn new(stops: &[SvgGradientStop]) -> Self {
        let min_gap = stops
            .windows(2)
            .map(|w| w[1].offset - w[0].offset)
            .filter(|gap| *gap > 0.0)
            .fold(1.0, f32::min);
        let size = ((1.0 / min_gap).ceil() as usize)
            .clamp(MIN_LUT_SIZE, MAX_LUT_SIZE)
            .next_power_of_two();

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
    function: &ShadingFunction,
    [t0, t1]: [f32; 2],
    pattern: &EncodedShadingPattern,
    tolerance: f32,
) -> Vec<SvgGradientStop> {
    let dt = t1 - t0;
    let mut bounds = function.stitching_bounds();
    if t1 < t0 {
        bounds.reverse();
    }
    let ends = bounds
        .into_iter()
        .filter_map(|bound| stitching_offset(bound, t0, t1).map(|offset| (bound, offset)));
    let mut start = t0;
    let mut start_offset = 0.0;
    let mut stops = Vec::new();
    for (end, end_offset) in ends.chain([(t1, 1.0)]) {
        let interval = (start != t0 || end != t1).then_some([start, end]);
        let sample = |t, interval| pattern.eval(function, &[t], interval);
        let start_color = sample(start, if start == t0 { None } else { interval });
        stops.push(SvgGradientStop {
            offset: start_offset,
            color: start_color,
        });

        let end_color = sample(end, if end == t1 { None } else { interval });

        approximate_gradient_interval(
            &mut |offset| sample(t0 + dt * offset, interval),
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
        start = end;
        start_offset = end_offset;
    }

    stops
}

fn stitching_offset(bound: f32, start: f32, end: f32) -> Option<f32> {
    let offset =
        ((f64::from(bound) - f64::from(start)) / (f64::from(end) - f64::from(start))) as f32;
    (0.0 < offset && offset < 1.0).then_some(offset)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaColor, ColorSpace};
    use crate::encode::{EncodedDummyShading, EncodedShadingType};
    use crate::function::Function;
    use hayro_syntax::object::{FromBytes, Object};

    #[test]
    fn stitching_offset_extremes() {
        assert_eq!(stitching_offset(0.0, -f32::MAX, f32::MAX), Some(0.5));
        assert_eq!(stitching_offset(0.0, -f32::MAX, f32::INFINITY), None);
        assert_eq!(stitching_offset(f32::NAN, 0.0, 1.0), None);
    }

    #[test]
    fn stitching_hard_stops() {
        let black = "<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [0] /N 1 >>";
        let white = "<< /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [1] /N 1 >>";
        let step = format!(
            "<< /FunctionType 3 /Domain [0 1] /Bounds [.25 .25]
            /Functions [{black} {black} {white}] /Encode [0 1 0 1 0 1] >>"
        );
        let child = step.replace("[.25 .25]", "[.75 .75]");
        let reversed = format!(
            "<< /FunctionType 3 /Domain [0 1] /Bounds []
            /Functions [{child}] /Encode [1 0] >>"
        );
        let adjacent = step.replace(
            "/Domain [0 1] /Bounds [.25 .25]",
            "/Domain [16777216 16777220] /Bounds [16777218 16777218]",
        );
        let pattern = EncodedShadingPattern {
            base_transform: Affine::IDENTITY,
            color_space: ColorSpace::device_gray(),
            background_color: AlphaColor::BLACK,
            shading_type: EncodedShadingType::Dummy(EncodedDummyShading),
            opacity: 1.0,
            transfer_function: None,
        };
        for (pdf, domain, offset, left, right) in [
            (&step, [0.0, 1.0], 0.25, 0.0, 1.0),
            (&step, [1.0, 0.0], 0.75, 1.0, 0.0),
            (&reversed, [0.0, 1.0], 0.25, 1.0, 0.0),
            (&reversed, [1.0, 0.0], 0.75, 0.0, 1.0),
            (&adjacent, [16777216.0, 16777220.0], 0.5, 0.0, 1.0),
        ] {
            let object = Object::from_bytes(pdf.as_bytes()).unwrap();
            let function = ShadingFunction::Single(Function::new(&object).unwrap());
            let stops = approximate_gradient_stops(&function, domain, &pattern, 0.01);
            let actual: Vec<_> = stops.iter().map(|s| (s.offset, s.color[0])).collect();
            assert_eq!(
                actual,
                [(0.0, left), (offset, left), (offset, right), (1.0, right)]
            );
        }

        let narrow = format!(
            "<< /FunctionType 3 /Domain [0 1]
            /Bounds [0.00000000000000000001 0.00000000000000000002]
            /Functions [{black} {white} {black}] /Encode [0 1 0 1 0 1] >>"
        );
        let object = Object::from_bytes(narrow.as_bytes()).unwrap();
        let function = ShadingFunction::Single(Function::new(&object).unwrap());
        let stops = approximate_gradient_stops(&function, [0.0, 1.0], &pattern, 0.01);
        assert_eq!(
            stops.iter().map(|s| s.color[0]).collect::<Vec<_>>(),
            [0.0, 0.0, 1.0, 1.0, 0.0, 0.0]
        );
        assert_eq!(ColorLut::new(&stops).colors.len(), MAX_LUT_SIZE);
    }
}
