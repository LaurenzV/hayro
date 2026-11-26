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
#[derive(Clone, Debug)]
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

        let stops = self.native_stops(pattern, tolerance)?;

        let transform = pattern.base_transform.inverse();
        let stops = pad_stops(stops, range, background)?;
        let mut max_slope = 1.0_f32;
        for pair in stops.windows(2) {
            let [a, b] = [&pair[0], &pair[1]];
            let gap = b.offset - a.offset;
            let rounding = 0.5 * f32::EPSILON * b.offset.abs().max(1.);
            if gap > 0. {
                let change = color_error(a.color, b.color);
                if change * rounding > tolerance * gap {
                    return None;
                }
                max_slope = max_slope.max(change / gap);
            }
        }
        let kind = self.gradient_kind(range, tolerance / max_slope)?;
        let span = match kind {
            SvgGradientKind::Linear { start, end } => (transform * end - transform * start).hypot(),
            SvgGradientKind::Radial {
                start_center,
                start_radius,
                end_center,
                end_radius,
            } => {
                if end_radius == 0.0
                    || (start_center.x as f32 == end_center.x as f32
                        && start_center.y as f32 == end_center.y as f32
                        && start_radius == end_radius)
                {
                    return None;
                }
                let [a, b, c, d, ..] = transform.as_coeffs();
                (transform * end_center - transform * start_center).hypot()
                    + f64::from(end_radius - start_radius).abs() * a.hypot(b).hypot(c.hypot(d))
            }
        };
        if span * f64::from(f32::EPSILON) >= 0.5
            && stops
                .windows(2)
                .any(|w| w[0].offset == w[1].offset && w[0].color != w[1].color)
        {
            return None;
        }
        Some(SvgGradient {
            kind,
            transform,
            stops,
        })
    }

    fn native_stops(
        &self,
        pattern: &EncodedShadingPattern,
        tolerance: f32,
    ) -> Option<Vec<SvgGradientStop>> {
        if let Some(p) = pattern.prepared(self) {
            let (tol, stops) = p
                .stops
                .get_or_init(|| (tolerance, self.derive_stops(pattern, tolerance)));
            if *tol == tolerance {
                return stops.clone();
            }
        }
        self.derive_stops(pattern, tolerance)
    }

    fn derive_stops(
        &self,
        pattern: &EncodedShadingPattern,
        tolerance: f32,
    ) -> Option<Vec<SvgGradientStop>> {
        if !tolerance.is_finite() || tolerance < 0.0 {
            return None;
        }
        let direct_color =
            pattern.color_space.is_device_gray_or_rgb() && pattern.transfer_function.is_none();
        if direct_color && tolerance < 1.0 / 255.0 {
            return None;
        }

        let mut stops: Vec<SvgGradientStop> = Vec::new();
        let mut emit = |start: f32, end: f32, ca: [f32; 4], cb: [f32; 4]| {
            if stops.last().is_none_or(|last| last.color != ca) {
                stops.push(SvgGradientStop {
                    offset: start,
                    color: ca,
                });
            }
            if let [.., left, middle] = &stops[..] {
                let width = f64::from(end) - f64::from(left.offset);
                let span = f64::from(middle.offset) - f64::from(left.offset);
                if left.offset < middle.offset
                    && middle.offset < end
                    && (0..4).all(|i| {
                        (f64::from(middle.color[i]) - f64::from(left.color[i])) * width
                            == (f64::from(cb[i]) - f64::from(left.color[i])) * span
                    })
                {
                    stops.pop();
                }
            }
            stops.push(SvgGradientStop {
                offset: end,
                color: cb,
            });
            (stops.len() <= crate::function::MAX_LINEAR_PIECES * 2).then_some(())
        };
        let cost = pattern
            .color_space
            .eval_ops()
            .saturating_add(self.function.ops_per_eval());
        let remaining = std::cell::Cell::new(crate::encode::MAX_PREP_OPS / cost);
        let convert = |values: &[f32]| {
            remaining.set(remaining.get().checked_sub(1)?);
            Some(pattern.convert(values))
        };
        self.function.linear_pieces(
            self.domain.map(f64::from),
            direct_color.then_some(tolerance),
            |[start, end], a, b| {
                if start >= end {
                    return None;
                }
                let (ca, cb) = (convert(a)?, convert(b)?);
                if !direct_color {
                    let sample = |t, color, values: &[f32]| ColorSample {
                        t,
                        color,
                        codes: pattern.color_space.encode_values(values),
                        inputs: smallvec::SmallVec::from_slice(values),
                    };
                    let mid = |t| {
                        let [lo, hi] = self.domain.map(f64::from);
                        let mut out = [0.0; crate::function::MAX_COMPONENTS];
                        let out = &mut out[..self.function.num_outputs()];
                        self.function
                            .eval_1d_f64(lo + (hi - lo) * f64::from(t), out)?;
                        Some(sample(t, convert(out)?, out))
                    };
                    return emit_color_interval(
                        &sample(start, ca, a),
                        &sample(end, cb, b),
                        &mid,
                        &mut emit,
                        &convert,
                        tolerance,
                    );
                }
                emit(start, end, ca, cb)
            },
        )?;

        Some(stops)
    }

    // SVG gradients pad with their end stops, so a non-extended end gets a short stretch of background.
    fn padded_range(&self, has_background: bool) -> Option<[f64; 2]> {
        let mut padding = self.extend.map(|extend| if extend { 0.0 } else { PADDING });

        if let RadialAxialParams::Radial { p1, r } = self.params {
            if r.x == 0.0 && r.y == 0.0 {
                return None;
            }
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

        let end = 1.0 + padding[1];
        (padding[1] == 0. || end > 1.).then_some([-padding[0], end])
    }

    fn gradient_kind(&self, [t0, t1]: [f64; 2], tolerance: f32) -> Option<SvgGradientKind> {
        Some(match &self.params {
            RadialAxialParams::Axial { .. } => SvgGradientKind::Linear {
                start: Point::new(t0, 0.0),
                end: Point::new(t1, 0.0),
            },
            RadialAxialParams::Radial { p1, r } => {
                let centers = [t0, t1].map(|t| (p1.to_vec2() * t).to_point());
                let radii = [t0, t1].map(|t| r.x + (r.y - r.x) * t);
                let rounded = radii.map(|r| r as f32);
                let span = radii[1] - radii[0];
                let budget = f64::from(tolerance) * span.abs();
                if (f64::from(rounded[1] - rounded[0]) - span).abs() > budget
                    || centers.iter().zip(radii).zip(rounded).any(|((p, r), f)| {
                        (p.x - f64::from(p.x as f32)).hypot(p.y - f64::from(p.y as f32))
                            + (r - f64::from(f)).abs()
                            > budget
                    })
                {
                    return None;
                }
                SvgGradientKind::Radial {
                    start_center: centers[0],
                    end_center: centers[1],
                    start_radius: rounded[0],
                    end_radius: rounded[1],
                }
            }
        })
    }
}

struct ColorSample {
    t: f32,
    color: [f32; 4],
    codes: smallvec::SmallVec<[u8; 4]>,
    inputs: smallvec::SmallVec<[f32; 4]>,
}

fn emit_color_interval(
    a: &ColorSample,
    b: &ColorSample,
    sample: &impl Fn(f32) -> Option<ColorSample>,
    emit: &mut impl FnMut(f32, f32, [f32; 4], [f32; 4]) -> Option<()>,
    convert: &impl Fn(&[f32]) -> Option<[f32; 4]>,
    tolerance: f32,
) -> Option<()> {
    let expected = |t| lerp_color(a.color, b.color, (t - a.t) / (b.t - a.t));
    if check_color_interval(a, b, sample, &expected, convert, tolerance).is_some() {
        return emit(a.t, b.t, a.color, b.color);
    }
    let t = a.t.midpoint(b.t);
    if t <= a.t || t >= b.t {
        return None;
    }
    let m = sample(t)?;
    emit_color_interval(a, &m, sample, emit, convert, tolerance)?;
    emit_color_interval(&m, b, sample, emit, convert, tolerance)
}

fn check_color_interval(
    a: &ColorSample,
    b: &ColorSample,
    sample: &impl Fn(f32) -> Option<ColorSample>,
    expected: &impl Fn(f32) -> [f32; 4],
    convert: &impl Fn(&[f32]) -> Option<[f32; 4]>,
    tolerance: f32,
) -> Option<()> {
    let (left, right) = (expected(a.t), expected(b.t));
    if color_error(a.color, left) > tolerance || color_error(b.color, right) > tolerance {
        return None;
    }
    // A monotone input crossing at most one byte boundary can only produce the two endpoint colors.
    let changes: u32 = a
        .codes
        .iter()
        .zip(&b.codes)
        .map(|(&x, &y)| u32::from(x.abs_diff(y)))
        .sum();
    if changes <= 1 && color_error(a.color, right).max(color_error(b.color, left)) <= tolerance {
        return Some(());
    }
    let t = a.t + (b.t - a.t) * 0.5;
    if t == a.t || t == b.t {
        // Between adjacent offsets each monotone component can only take codes in its endpoint range.
        let mut varying = smallvec::SmallVec::<[usize; 4]>::new();
        for (i, (&x, &y)) in a.codes.iter().zip(&b.codes).enumerate() {
            if x != y {
                if x.abs_diff(y) != 1 || varying.len() == 4 || i >= a.inputs.len() {
                    return None;
                }
                varying.push(i);
            }
        }
        let mut values = a.inputs.clone();
        for mask in 0..1 << varying.len() {
            for (bit, &i) in varying.iter().enumerate() {
                values[i] = if mask & (1 << bit) == 0 {
                    a.inputs[i]
                } else {
                    b.inputs[i]
                };
            }
            let color = convert(&values)?;
            if color_error(color, left).max(color_error(color, right)) > tolerance {
                return None;
            }
        }
        return Some(());
    }
    let m = sample(t)?;
    check_color_interval(a, &m, sample, expected, convert, tolerance)?;
    check_color_interval(&m, b, sample, expected, convert, tolerance)
}

pub(crate) fn function_lut(
    function: &ShadingFunction,
    range: [f32; 2],
    pattern: &EncodedShadingPattern,
) -> ColorLut {
    ColorLut::new(&approximate_gradient_stops(
        |t| sample_function(function, range, pattern, t),
        &normalized_stitching_bounds(function, range),
        LUT_TOLERANCE,
    ))
}

fn sample_function(
    function: &ShadingFunction,
    [t0, t1]: [f32; 2],
    pattern: &EncodedShadingPattern,
    t: f32,
) -> [f32; 4] {
    let [lo, hi] = [t0, t1].map(f64::from);
    pattern.eval_1d(function, lo + (hi - lo) * f64::from(t))
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
) -> Option<Vec<SvgGradientStop>> {
    let padding = |offset| SvgGradientStop {
        offset,
        color: background.unwrap_or([0.; 4]),
    };
    if t0 < 0. {
        stops.splice(0..0, [padding(0.), padding(0.)]);
    }
    if t1 > 1. {
        stops.extend([padding(1.), padding(1.)]);
    }

    let end = stops.len() - 1;
    let mut previous = None;
    for (i, stop) in stops.iter_mut().enumerate() {
        let position = match i {
            0 => t0,
            i if i == end => t1,
            _ => f64::from(stop.offset),
        };
        let offset = ((position - t0) / (t1 - t0)) as f32;
        if let Some((last, left)) = previous
            && position != last
        {
            // usvg merges within four ULPs and moves duplicates left by EPSILON (right at zero).
            let rightmost = if left == 0. { f32::EPSILON } else { left };
            if offset.to_bits().saturating_sub(left.to_bits()) <= 4
                || offset - f32::EPSILON <= rightmost
            {
                return None;
            }
        }
        stop.offset = offset;
        previous = Some((position, offset));
    }
    Some(stops)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaColor, ColorSpace};
    use crate::encode::EncodedShadingType;
    use crate::function::Function;
    use hayro_syntax::object::{Dict, FromBytes, Object};

    #[test]
    fn zero_radius_circles_paint_no_pixels() {
        let dict = Dict::from_bytes(b"<< /Shading << /ShadingType 3 /Coords [0 0.5 0 1 0.5 0] /Extend [true true] /ColorSpace /DeviceGray /Function << /FunctionType 2 /Domain [0 1] /N 1 >> >> >>").unwrap();
        let pattern = crate::pattern::ShadingPattern::new(&dict, &crate::cache::Cache::default())
            .unwrap()
            .encode();
        let EncodedShadingType::RadialAxial(g) = &pattern.shading_type else {
            panic!()
        };
        assert!(g.as_svg_gradient(&pattern, 0.01).is_none());
        let texture = pattern.render_texture(kurbo::Rect::new(0., 0., 3., 1.), 1.);
        assert_eq!(texture.data, [0; 12]);
    }

    #[test]
    fn converted_exponential_emits_certified_intermediate_stops() {
        let dict = Dict::from_bytes(b"<< /Shading << /ShadingType 2 /Coords [0 0 1 0] /Extend [true true] /ColorSpace /DeviceCMYK /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0 0 1] /N 2 >> >> >>").unwrap();
        let pattern = crate::pattern::ShadingPattern::new(&dict, &crate::cache::Cache::default())
            .unwrap()
            .encode();
        let EncodedShadingType::RadialAxial(g) = &pattern.shading_type else {
            panic!()
        };
        let tolerance = 0.02;
        let stops = g.native_stops(&pattern, tolerance).unwrap();
        assert!(stops.len() > 2);
        for pair in stops.windows(2) {
            for i in 0..=128 {
                let u = i as f32 / 128.;
                let t = pair[0].offset + u * (pair[1].offset - pair[0].offset);
                let color = pattern.eval_1d(&g.function, f64::from(t));
                assert!(
                    color_error(color, lerp_color(pair[0].color, pair[1].color, u)) <= tolerance
                );
            }
        }
    }

    #[test]
    fn adjacent_offsets_with_two_changed_ink_codes_are_unresolved() {
        let sample = |t, code| ColorSample {
            t,
            color: [0.0; 4],
            codes: smallvec::smallvec![code, code],
            inputs: smallvec::smallvec![code as f32, code as f32],
        };
        let a = sample(0.5, 127);
        let b = sample(a.t.next_up(), 128);
        assert!(
            check_color_interval(
                &a,
                &b,
                &|_| unreachable!(),
                &|_| a.color,
                &|c| Some([f32::from(c[0] != c[1]); 4]),
                0.01
            )
            .is_none()
        );
    }

    fn stops(dict: &[u8]) -> Option<Vec<SvgGradientStop>> {
        let function = Function::new(&Object::Dict(Dict::from_bytes(dict).unwrap())).unwrap();
        let pattern = EncodedShadingPattern {
            colors: std::sync::Arc::default(),
            base_transform: Affine::IDENTITY,
            color_space: ColorSpace::device_gray(),
            background_color: AlphaColor::TRANSPARENT,
            shading_type: EncodedShadingType::RadialAxial(EncodedRadialAxialShading {
                params: RadialAxialParams::Axial {
                    sampling_transform: Affine::IDENTITY,
                    expected_base_transform: Affine::IDENTITY,
                    start: Point::ZERO,
                    axis: kurbo::Vec2::new(1.0, 0.0),
                },
                domain: [0.0, 1.0],
                function: ShadingFunction::Single(function),
                extend: [true; 2],
            }),
            opacity: 1.0,
            transfer_function: None,
        };
        let EncodedShadingType::RadialAxial(shading) = &pattern.shading_type else {
            unreachable!()
        };
        Some(shading.as_svg_gradient(&pattern, 0.01)?.stops)
    }

    #[test]
    fn linear_and_stitched_gradients_keep_native_stops() {
        let line = "<< /FunctionType 2 /Domain [0 1] /N 1 >>";
        assert_eq!(stops(line.as_bytes()).unwrap().len(), 2);
        let constant = "<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 0 >>";
        assert_eq!(
            stops(constant.as_bytes())
                .unwrap()
                .iter()
                .map(|s| (s.offset, s.color[0]))
                .collect::<Vec<_>>(),
            [(0.0, 1.0), (1.0, 1.0)]
        );
        let stitch = |bound, encode| {
            format!(
                "<< /FunctionType 3 /Domain [0 1] /Bounds [{bound}] /Encode [{encode}] /Functions [{line} {line}] >>"
            )
        };
        let ramp = stops(stitch("0.5", "0 1 1 0").as_bytes()).unwrap();
        assert_eq!(
            ramp.iter()
                .map(|s| (s.offset, s.color[0]))
                .collect::<Vec<_>>(),
            [(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)]
        );
        assert!(stops(stitch("0.00001", "0 1 1 0").as_bytes()).is_some());
        let jump = stops(stitch("0.5", "0 1 0 1").as_bytes()).unwrap();
        assert_eq!(
            jump.iter()
                .map(|s| (s.offset, s.color[0]))
                .collect::<Vec<_>>(),
            [(0.0, 0.0), (0.5, 1.0), (0.5, 0.0), (1.0, 1.0)]
        );
    }
}
