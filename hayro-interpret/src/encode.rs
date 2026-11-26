//! Encoding shading patterns for easy sampling.

use crate::canvas::Canvas;
use crate::color::{AlphaColor, ColorSpace};
use crate::function::MAX_COMPONENTS;
use crate::function_shading::draw_function;
use crate::interpret::state::ActiveTransferFunction;
use crate::mesh::draw_mesh;
use crate::pattern::ShadingPattern;
use crate::shading::{ShadingFunction, ShadingType};
use kurbo::{Affine, Point, Rect, Vec2};
use std::sync::{Arc, OnceLock};

/// A shading pattern that was encoded so it can be sampled.
#[derive(Clone, Debug)]
pub struct EncodedShadingPattern {
    pub(crate) colors: Arc<OnceLock<PreparedColors>>,
    /// The base transform of the shading pattern.
    pub base_transform: Affine,
    pub(crate) color_space: ColorSpace,
    pub(crate) background_color: AlphaColor,
    /// The encoded shading type.
    pub shading_type: EncodedShadingType,
    pub(crate) opacity: f32,
    pub(crate) transfer_function: Option<ActiveTransferFunction>,
}

/// The encoded shading type.
#[derive(Clone, Debug)]
pub enum EncodedShadingType {
    /// Function-based shading.
    FunctionBased(EncodedFunctionBasedShading),
    /// Axial or radial shading.
    RadialAxial(EncodedRadialAxialShading),
    /// Triangle-mesh or patch-mesh shading.
    Mesh(EncodedMeshShading),
    /// Empty dummy shading.
    Dummy(EncodedDummyShading),
}

/// Encoded function-based shading.
#[derive(Clone, Debug)]
pub struct EncodedFunctionBasedShading {
    pub(crate) domain: Rect,
    pub(crate) function: ShadingFunction,
}

/// Encoded axial or radial shading.
#[derive(Clone, Debug)]
pub struct EncodedRadialAxialShading {
    pub(crate) function: ShadingFunction,
    pub(crate) params: RadialAxialParams,
    pub(crate) domain: [f32; 2],
    pub(crate) extend: [bool; 2],
}

/// Encoded triangle-mesh or patch-mesh shading.
#[derive(Clone, Debug)]
pub struct EncodedMeshShading {
    pub(crate) shading_type: Arc<ShadingType>,
}

/// Encoded dummy shading.
#[derive(Clone, Debug)]
pub struct EncodedDummyShading;

/// A shading that was rasterized into a texture.
pub struct ShadingTexture {
    /// The straight (non-premultiplied) RGBA8 pixel data, row-major.
    pub data: Vec<u8>,
    /// The width of the texture in pixels.
    pub width: u32,
    /// The height of the texture in pixels.
    pub height: u32,
    /// The transform from texture space to the space of the bounding box.
    pub transform: Affine,
    /// Whether any pixel of the texture is not fully opaque.
    pub has_transparency: bool,
}

/// The texture dimensions for `bbox` at `scale` pixels per unit.
pub fn texture_dimensions(bbox: Rect, scale: f32) -> (u32, u32) {
    (
        (bbox.width() as f32 * scale).max(1.0).ceil() as u32,
        (bbox.height() as f32 * scale).max(1.0).ceil() as u32,
    )
}

#[derive(Debug)]
pub(crate) struct PreparedColors {
    function: ShadingFunction,
    domain: [f32; 2],
    space: ColorSpace,
    opacity: f32,
    runs: OnceLock<Vec<ColorRun>>,
    pub(crate) stops: OnceLock<(f32, Option<Vec<crate::gradient::SvgGradientStop>>)>,
}

#[derive(Debug)]
struct ColorRun {
    first: f64,
    last: f64,
    color: [f32; 4],
}

impl EncodedShadingPattern {
    pub(crate) fn prepared(&self, g: &EncodedRadialAxialShading) -> Option<&PreparedColors> {
        if self.transfer_function.is_some() {
            return None;
        }
        let p = self.colors.get_or_init(|| PreparedColors {
            function: g.function.clone(),
            domain: g.domain,
            space: self.color_space.clone(),
            opacity: self.opacity,
            runs: OnceLock::new(),
            stops: OnceLock::new(),
        });
        (p.function.same(&g.function)
            && p.domain == g.domain
            && p.space.same(&self.color_space)
            && p.opacity.to_bits() == self.opacity.to_bits())
        .then_some(p)
    }

    pub(crate) fn finish(&self, mut color: [f32; 4]) -> [f32; 4] {
        color[3] *= self.opacity;

        match &self.transfer_function {
            Some(tf) => tf.apply(&AlphaColor::new(color)).components(),
            None => color,
        }
    }

    pub(crate) fn convert(&self, components: &[f32]) -> [f32; 4] {
        self.finish(self.color_space.to_rgba(components, 1.0).components())
    }

    pub(crate) fn eval_1d(&self, function: &ShadingFunction, x: f64) -> [f32; 4] {
        self.eval_color(function, |out| function.eval_1d_f64(x, out))
    }

    pub(crate) fn eval_color(
        &self,
        function: &ShadingFunction,
        eval: impl FnOnce(&mut [f32]) -> Option<()>,
    ) -> [f32; 4] {
        let mut out = [0.0; MAX_COMPONENTS];
        let out = &mut out[..function.num_outputs()];

        match eval(out) {
            Some(()) => self.convert(out),
            None => [0.0; 4],
        }
    }

    /// Call `f` with the straight RGBA color of each pixel of a grid covering `bbox`, in row-major order.
    pub fn sample_texture(
        &self,
        bbox: Rect,
        scale: f32,
        f: impl FnMut([f32; 4]),
    ) -> (u32, u32, Affine) {
        let (width, height) = texture_dimensions(bbox, scale);
        let grid_transform =
            Affine::translate((bbox.x0, bbox.y0)) * Affine::scale(1.0 / scale as f64);
        if let EncodedShadingType::RadialAxial(shading) = &self.shading_type {
            let sampling_transform = match shading.params {
                RadialAxialParams::Axial {
                    start,
                    axis,
                    sampling_transform,
                    expected_base_transform,
                } => {
                    if self.base_transform == expected_base_transform {
                        sampling_transform
                    } else {
                        Affine::new([axis.x, axis.y, -axis.y, axis.x, start.x, start.y])
                            * self.base_transform
                    }
                }
                RadialAxialParams::Radial { .. } => self.base_transform,
            };
            shading.sample(width, height, sampling_transform * grid_transform, self, f);
            return (width, height, grid_transform);
        }
        let mut canvas = Canvas::new(
            width,
            height,
            self.finish(self.background_color.components()),
        );
        let transform = grid_transform.inverse() * self.base_transform.inverse();

        match &self.shading_type {
            EncodedShadingType::FunctionBased(shading) => {
                draw_function(
                    &mut canvas,
                    shading.domain,
                    &shading.function,
                    transform,
                    self,
                );
            }
            EncodedShadingType::Mesh(shading) => {
                draw_mesh(&mut canvas, &shading.shading_type, transform, self);
            }
            EncodedShadingType::Dummy(_) => {}
            EncodedShadingType::RadialAxial(_) => unreachable!(),
        }

        canvas.pixels.into_iter().for_each(f);

        (width, height, grid_transform)
    }

    /// Rasterize the shading into a straight RGBA8 texture covering `bbox`.
    pub fn render_texture(&self, bbox: Rect, scale: f32) -> ShadingTexture {
        let (width, height) = texture_dimensions(bbox, scale);
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        let mut has_transparency = false;

        let (_, _, transform) = self.sample_texture(bbox, scale, |sample| {
            let converted = [
                (sample[0] * 255.0 + 0.5) as u8,
                (sample[1] * 255.0 + 0.5) as u8,
                (sample[2] * 255.0 + 0.5) as u8,
                (sample[3] * 255.0 + 0.5) as u8,
            ];

            has_transparency |= converted[3] != 255;
            data.extend_from_slice(&converted);
        });

        ShadingTexture {
            data,
            width,
            height,
            transform,
            has_transparency,
        }
    }
}

impl ShadingPattern {
    /// Encode the shading pattern.
    pub fn encode(&self) -> EncodedShadingPattern {
        let base_transform;

        let shading_type = match self.shading.shading_type.as_ref() {
            ShadingType::FunctionBased {
                domain,
                matrix,
                function,
            } => {
                base_transform = (self.matrix * *matrix).inverse();
                encode_function_shading(domain, function)
            }
            ShadingType::RadialAxial {
                coords,
                domain,
                function,
                extend,
                axial,
            } => {
                let sampling_transform = self.matrix.inverse();
                let (encoded, initial_transform) = encode_axial_shading(
                    self.shading.coordinates(*coords),
                    *domain,
                    function,
                    *extend,
                    *axial,
                    sampling_transform,
                );

                base_transform = initial_transform * sampling_transform;

                encoded
            }
            ShadingType::TriangleMesh { .. } | ShadingType::PatchMesh { .. } => {
                base_transform = self.matrix.inverse();

                EncodedShadingType::Mesh(EncodedMeshShading {
                    shading_type: self.shading.shading_type.clone(),
                })
            }
            ShadingType::Dummy => {
                base_transform = Affine::IDENTITY;

                EncodedShadingType::Dummy(EncodedDummyShading)
            }
        };

        let color_space = self.shading.color_space.clone();

        let background_color = match (&shading_type, &self.shading.background) {
            (EncodedShadingType::Dummy(_), _) | (_, None) => AlphaColor::TRANSPARENT,
            (_, Some(background)) => color_space.to_rgba(background, 1.0),
        };

        EncodedShadingPattern {
            colors: self.shading.colors.clone(),
            color_space,
            background_color,
            shading_type,
            base_transform,
            opacity: self.opacity,
            transfer_function: self.transfer_function.clone(),
        }
    }
}

fn encode_axial_shading(
    coords: [f64; 6],
    domain: [f32; 2],
    function: &ShadingFunction,
    extend: [bool; 2],
    is_axial: bool,
    sampling_transform: Affine,
) -> (EncodedShadingType, Affine) {
    let initial_transform;

    let params = if is_axial {
        let [x_0, y_0, x_1, y_1, _, _] = coords;

        let start = Point::new(x_0, y_0);
        let axis = Point::new(x_1, y_1) - start;
        initial_transform =
            Affine::new([axis.x, axis.y, -axis.y, axis.x, start.x, start.y]).inverse();
        RadialAxialParams::Axial {
            start,
            axis,
            sampling_transform,
            expected_base_transform: initial_transform * sampling_transform,
        }
    } else {
        let [x_0, y_0, r0, x_1, y_1, r_1] = coords;

        initial_transform = Affine::translate((-x_0, -y_0));
        let new_x1 = x_1 - x_0;
        let new_y1 = y_1 - y_0;

        let p1 = Point::new(new_x1, new_y1);
        let r = Point::new(r0, r_1);

        RadialAxialParams::Radial { p1, r }
    };

    (
        EncodedShadingType::RadialAxial(EncodedRadialAxialShading {
            function: function.clone(),
            params,
            domain,
            extend,
        }),
        initial_transform,
    )
}

fn encode_function_shading(domain: &[f32; 4], function: &ShadingFunction) -> EncodedShadingType {
    let domain = Rect::new(
        domain[0] as f64,
        domain[2] as f64,
        domain[1] as f64,
        domain[3] as f64,
    );

    EncodedShadingType::FunctionBased(EncodedFunctionBasedShading {
        domain,
        function: function.clone(),
    })
}

#[derive(Clone, Debug)]
pub(crate) enum RadialAxialParams {
    Axial {
        start: Point,
        axis: Vec2,
        sampling_transform: Affine,
        expected_base_transform: Affine,
    },
    Radial {
        p1: Point,
        r: Point,
    },
}

pub(crate) const MAX_PREP_OPS: usize = 2_000_000;

impl EncodedRadialAxialShading {
    /// Return a single device row or column when the axial parameter is invariant on that axis.
    pub fn texture_bounds(&self, bbox: Rect, pattern: &EncodedShadingPattern) -> Rect {
        let RadialAxialParams::Axial {
            sampling_transform,
            expected_base_transform,
            ..
        } = self.params
        else {
            return bbox;
        };
        if pattern.base_transform != expected_base_transform {
            return bbox;
        }
        if self.invariant_y(sampling_transform) {
            Rect::new(bbox.x0, bbox.y0, bbox.x1, bbox.y0 + 1.)
        } else if self.invariant_x(sampling_transform) {
            Rect::new(bbox.x0, bbox.y0, bbox.x0 + 1., bbox.y1)
        } else {
            bbox
        }
    }

    fn color_runs(&self, pattern: &EncodedShadingPattern) -> Vec<ColorRun> {
        let [a, b] = self.domain.map(f64::from);
        let x = |t| a + (b - a) * t;
        let mut runs: Vec<ColorRun> = Vec::new();
        let mut pending: smallvec::SmallVec<[(f64, f64, u8); 16]> =
            smallvec::smallvec![(0., 1., 0)];
        let mut values = [0.; MAX_COMPONENTS];
        let values = &mut values[..self.function.num_outputs()];
        let pair_cost = self
            .function
            .ops_per_eval()
            .saturating_mul(2)
            .saturating_add(pattern.color_space.eval_ops());
        let mut remaining_pairs = MAX_PREP_OPS / pair_cost;
        let mut budget = 16_384;
        while let Some((first, last, depth)) = pending.pop() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let range = [x(first), x(last)];
            match self.function.monotone(range) {
                None => break,
                Some(true) => {
                    if remaining_pairs == 0 {
                        break;
                    }
                    remaining_pairs -= 1;
                    if self.function.eval_1d_f64(range[0], values).is_none() {
                        break;
                    }
                    let codes = pattern.color_space.encode_values(values);
                    if self.function.eval_1d_f64(range[1], values).is_none() {
                        break;
                    }
                    if codes == pattern.color_space.encode_values(values) {
                        let color = pattern.convert(values);
                        if let Some(prev) = runs.last_mut()
                            && prev.last == first
                            && prev.color == color
                        {
                            prev.last = last;
                        } else {
                            runs.push(ColorRun { first, last, color });
                        }
                        continue;
                    }
                }
                Some(false) => {}
            }
            if depth < 18 {
                let mid = first.midpoint(last);
                pending.push((mid, last, depth + 1));
                pending.push((first, mid, depth + 1));
            }
        }
        runs
    }

    fn sample(
        &self,
        width: u32,
        height: u32,
        transform: Affine,
        pattern: &EncodedShadingPattern,
        mut emit: impl FnMut([f32; 4]),
    ) {
        let background = pattern.finish(pattern.background_color.components());
        let [start, end] = self.domain.map(f64::from);
        let samples = if self.invariant_y(transform) {
            u64::from(width)
        } else if self.invariant_x(transform) {
            u64::from(height)
        } else {
            u64::from(width) * u64::from(height)
        };
        let local = OnceLock::new();
        let cache = pattern.prepared(self).map_or(&local, |p| &p.runs);
        let mut runs = cache.get().map(Vec::as_slice);
        if samples < 4096 && runs.is_none() {
            runs = Some(&[]);
        }
        let mut interior_samples = 0;
        let mut last_run: Option<&ColorRun> = None;
        let (mut start_color, mut end_color) = (None, None);
        let mut sample = |x: u32, y: u32| {
            let point = transform * Point::new(x as f64 + 0.5, y as f64 + 0.5);
            self.position(point)
                .map(|t| match t {
                    0.0 => {
                        *start_color.get_or_insert_with(|| pattern.eval_1d(&self.function, start))
                    }
                    1.0 => *end_color.get_or_insert_with(|| pattern.eval_1d(&self.function, end)),
                    _ => {
                        if runs.is_none() {
                            interior_samples += 1;
                            if interior_samples < 16 {
                                return pattern.eval_1d(&self.function, start + (end - start) * t);
                            }
                        }
                        let runs = *runs.get_or_insert_with(|| {
                            cache.get_or_init(|| self.color_runs(pattern)).as_slice()
                        });
                        if runs.is_empty() {
                            return pattern.eval_1d(&self.function, start + (end - start) * t);
                        }
                        if let Some(run) = last_run
                            && t >= run.first
                            && t <= run.last
                        {
                            return run.color;
                        }
                        last_run = runs.get(runs.partition_point(|run| run.last < t));
                        last_run.filter(|run| t >= run.first).map_or_else(
                            || pattern.eval_1d(&self.function, start + (end - start) * t),
                            |run| run.color,
                        )
                    }
                })
                .unwrap_or(background)
        };
        if height > 1 && self.invariant_y(transform) {
            let row = (0..width).map(|x| sample(x, 0)).collect::<Vec<_>>();
            for _ in 0..height {
                row.iter().copied().for_each(&mut emit);
            }
        } else if self.invariant_x(transform) {
            for y in 0..height {
                std::iter::repeat_n(sample(0, y), width as usize).for_each(&mut emit);
            }
        } else {
            for y in 0..height {
                for x in 0..width {
                    emit(sample(x, y));
                }
            }
        }
    }

    pub(crate) fn invariant_y(&self, transform: Affine) -> bool {
        let [_, _, c, d, ..] = transform.as_coeffs();
        matches!(self.params, RadialAxialParams::Axial { axis, .. } if (axis.x == 0. || c == 0.) && (axis.y == 0. || d == 0.))
    }

    pub(crate) fn invariant_x(&self, transform: Affine) -> bool {
        let [a, b, ..] = transform.as_coeffs();
        matches!(self.params, RadialAxialParams::Axial { axis, .. } if (axis.x == 0. || a == 0.) && (axis.y == 0. || b == 0.))
    }

    fn position(&self, pos: Point) -> Option<f64> {
        let t = match self.params {
            RadialAxialParams::Axial { start, axis, .. } => (pos - start).dot(axis) / axis.hypot2(),
            RadialAxialParams::Radial { p1, r } => radial_pos(pos, p1, r, self.extend)?,
        };

        if !t.is_finite() {
            return None;
        }
        if t < 0.0 {
            self.extend[0].then_some(0.0)
        } else if t > 1.0 {
            self.extend[1].then_some(1.0)
        } else {
            Some(t)
        }
    }
}

fn radial_pos(pos: Point, p1: Point, r: Point, extend: [bool; 2]) -> Option<f64> {
    if p1 == Point::ORIGIN && r.x == 0. && r.y > 0. {
        let squared = pos.to_vec2().hypot2();
        let distance = if squared.is_normal() {
            squared.sqrt()
        } else {
            pos.x.hypot(pos.y)
        };
        let t = distance / r.y;
        if extend[1] || (t - 1.).abs() > 4. * f64::EPSILON {
            return (extend[1] || t <= 1.).then_some(t);
        }
    }
    if r.x == 0.0 && r.y == 0.0 {
        return None;
    }
    let dr = r.y - r.x;
    let r0 = r.x;
    let a = norm_squared_minus_square(p1, dr);
    let bx = pos.x * p1.x;
    let by = pos.y * p1.y;
    let br = r0 * dr;
    // Add the smallest product last so cancellation can expose it.
    let b = if bx.abs() <= by.abs() && bx.abs() <= br.abs() {
        (by + br) + bx
    } else if by.abs() <= br.abs() {
        (bx + br) + by
    } else {
        (bx + by) + br
    };
    let c = || norm_squared_minus_square(pos, r.x);
    let valid = |t: f64| (extend[0] || t >= 0.0) && (extend[1] || t <= 1.0) && r.x + dr * t >= 0.0;

    let refine_endpoint = |t: f64| {
        let (edge, residual, slope) = if !extend[1] && (t - 1.).abs() <= 4. * f64::EPSILON {
            (
                1.,
                norm_squared_minus_square((pos - p1).to_point(), r.y),
                -2. * ((pos - p1).dot(p1.to_vec2()) + r.y * dr),
            )
        } else if !extend[0] && t.abs() <= f64::EPSILON {
            (0., c(), -2. * b)
        } else {
            return t;
        };
        if residual == 0. {
            edge
        } else if slope != 0. {
            let delta = -residual / slope;
            if edge == 1. && delta > 0. {
                1.0_f64.next_up()
            } else {
                edge + delta
            }
        } else {
            t
        }
    };

    if a == 0.0 {
        if b == 0.0 {
            return [1.0, 0.0].into_iter().find(|&t| c() == 0.0 && valid(t));
        }
        let t = refine_endpoint(c() / (2.0 * b));
        return valid(t).then_some(t);
    }

    let (dx, dy) = (p1.x, p1.y);
    let (x, y) = (pos.x, pos.y);
    let u = r0 * dx + dr * x;
    let v = r0 * dy + dr * y;
    // Factor u² + v² − (x*dy − y*dx)² before the cross product loses its smaller term.
    let d = if u.abs() >= v.abs() {
        (dx * (r0 - y) + x * (dr + dy)) * (dx * (r0 + y) + x * (dr - dy)) + v * v
    } else {
        (dy * (r0 + x) + y * (dr - dx)) * (dy * (r0 - x) + y * (dr + dx)) + u * u
    };
    if d < 0.0 {
        return None;
    }
    let sqrt_d = d.sqrt();

    // Add terms of the same sign, then recover the other root from their product c/a.
    let q = b + sqrt_d.copysign(b);

    if q == 0.0 {
        return (c() == 0.0 && valid(0.0)).then_some(0.0);
    }

    let t = q / a;
    let other = refine_endpoint(c() / q);
    let scale = dx.abs().max(dy.abs()).max(dr.abs());
    // Six-digit focal-point rounding can turn a tangent cone into one with a spurious second root.
    if a > 0.0 && dr > 0.0 && extend[1] && (a / scale) / scale <= 2e-6 && valid(other) {
        return Some(other);
    }
    let t = refine_endpoint(t);
    let (max, min) = if q.is_sign_negative() == a.is_sign_negative() {
        (t, other)
    } else {
        (other, t)
    };
    if valid(max) {
        Some(max)
    } else {
        valid(min).then_some(min)
    }
}

fn norm_squared_minus_square(p: Point, r: f64) -> f64 {
    let (major, minor) = if p.x.abs() >= p.y.abs() {
        (p.x.abs(), p.y)
    } else {
        (p.y.abs(), p.x)
    };
    (major - r.abs()) * (major + r.abs()) + minor * minor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::shading::Shading;
    use hayro_syntax::object::{Dict, FromBytes};

    #[test]
    fn pixel_center_on_stitch_uses_the_right_piece() {
        let line = "<< /FunctionType 2 /Domain [0 1] /N 1 >>";
        let data = format!(
            "<< /Shading << /ShadingType 2 /Coords [0 0 1 0] /ColorSpace /DeviceGray /Function << /FunctionType 3 /Domain [0 1] /Functions [{line} {line}] /Bounds [0.5] /Encode [0 0 1 1] >> >> >>"
        );
        let dict = Dict::from_bytes(data.as_bytes()).unwrap();
        let pattern = ShadingPattern::new(&dict, &Cache::default())
            .unwrap()
            .encode();
        pattern.sample_texture(Rect::new(0., 0., 1., 1.), 1., |c| assert_eq!(c, [1.; 4]));
    }

    #[test]
    fn tiny_concentric_circles_survive_parsing() {
        let dict = Dict::from_bytes(b"<< /Shading << /ShadingType 3 /Coords [0 0 1 0 0 1.00000001] /ColorSpace /DeviceGray /Function << /FunctionType 2 /Domain [0 1] /N 1 >> >> >>").unwrap();
        let mut source = ShadingPattern::new(&dict, &Cache::default()).unwrap();
        let ShadingType::RadialAxial { extend, .. } =
            Arc::make_mut(&mut Arc::make_mut(&mut source.shading).shading_type)
        else {
            panic!()
        };
        *extend = [true; 2];
        let pattern = source.encode();
        let shading = Arc::make_mut(&mut source.shading);
        let ShadingType::RadialAxial {
            domain,
            function,
            extend,
            axial,
            ..
        } = shading.shading_type.as_ref()
        else {
            panic!()
        };
        shading.shading_type = Arc::new(ShadingType::RadialAxial {
            coords: [0_f32, 0., 0., 0., 0., 2.],
            domain: *domain,
            function: function.clone(),
            extend: *extend,
            axial: *axial,
        });
        let EncodedShadingType::RadialAxial(rebuilt) = source.encode().shading_type else {
            panic!()
        };
        assert_eq!(rebuilt.position(Point::new(1., 0.)), Some(0.5));
        let EncodedShadingType::RadialAxial(g) = pattern.shading_type else {
            panic!()
        };
        assert!((g.position(Point::new(1.000000005, 0.)).unwrap() - 0.5).abs() < 1e-6);
    }

    fn test_shading(body: &str, object: &str) -> ShadingPattern {
        use hayro_syntax::{Pdf, object::ObjectIdentifier};
        let data = format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n5 0 obj\n<< /ShadingType 2 {body} >>\nendobj\n6 0 obj\n{object}\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF"
        );
        let pdf = Pdf::new(data.into_bytes()).unwrap();
        let dict = pdf
            .xref()
            .get::<Dict<'_>>(ObjectIdentifier::new(5, 0))
            .unwrap();
        ShadingPattern {
            shading: Arc::new(Shading::new(&dict, None, &Cache::default()).unwrap()),
            matrix: Affine::IDENTITY,
            opacity: 1.,
            transfer_function: None,
        }
    }

    #[test]
    fn constant_runs_match_scalar_samples_and_stitch_ownership() {
        let peak = test_shading(
            "/Coords [0 0 100 0] /ColorSpace /DeviceGray /Function 6 0 R",
            "<< /FunctionType 0 /Domain [0 1] /Range [0 1] /Size [9] /BitsPerSample 8 /Length 9 >>\nstream\nAAAzAAAAA\nendstream",
        );
        let line = "<< /FunctionType 2 /Domain [0 1] /N 1 >>";
        let source = format!(
            "<< /Shading << /ShadingType 2 /Coords [0 0 1 0] /ColorSpace /DeviceGray /Function << /FunctionType 3 /Domain [0 1] /Functions [{line} {line} {line}] /Bounds [0.5 0.5] /Encode [0 0 0 1 1 0] >> >> >>"
        );
        let dict = Dict::from_bytes(source.as_bytes()).unwrap();
        let stitched = ShadingPattern::new(&dict, &Cache::default()).unwrap();
        for (source, end) in [(peak, 1024.), (stitched, 1.)] {
            let mut pattern = source.encode();
            let EncodedShadingType::RadialAxial(g) = &mut pattern.shading_type else {
                panic!()
            };
            g.domain = [0., end];
            let EncodedShadingType::RadialAxial(g) = &pattern.shading_type else {
                unreachable!()
            };
            let runs = g.color_runs(&pattern);
            assert!(!runs.is_empty());
            for run in runs {
                for t in [run.first, run.first.midpoint(run.last), run.last] {
                    assert_eq!(run.color, pattern.eval_1d(&g.function, f64::from(end) * t));
                }
            }
            let width = 65537;
            let scale = if end == 1. { 1. } else { 100. } / f64::from(width);
            let mut x = 0;
            g.sample(width, 1, Affine::scale(scale), &pattern, |c| {
                let t = g
                    .position(Point::new((f64::from(x) + 0.5) * scale, 0.))
                    .unwrap();
                assert_eq!(c, pattern.eval_1d(&g.function, f64::from(end) * t));
                x += 1;
            });
        }
    }

    #[test]
    fn sampled_tint_budget_counts_multilinear_corners() {
        let inks = (0..16).map(|i| format!("/Ink{i} ")).collect::<String>();
        let body = format!(
            "/Coords [0 0 1 0] /ColorSpace [/DeviceN [{inks}] /DeviceGray 6 0 R] /Function << /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N 1 >>",
            "0 ".repeat(16),
            "1 ".repeat(16)
        );
        let table = format!(
            "<< /FunctionType 0 /Domain [{}] /Range [0 1] /Size [{}] /BitsPerSample 1 /Length 8192 >>\nstream\n{}\nendstream",
            "0 1 ".repeat(16),
            "2 ".repeat(16),
            "A".repeat(8192)
        );
        let pattern = test_shading(&body, &table).encode();
        assert_eq!(MAX_PREP_OPS / pattern.color_space.eval_ops(), 30);
    }

    #[test]
    fn one_interior_pixel_skips_expensive_tint_preparation() {
        let program = format!("{{ pop {} }}", "dup pop ".repeat(500));
        let pattern = test_shading("/Coords [0 0 1 0] /Extend [true true] /ColorSpace [/DeviceN [/A /B] /DeviceGray 6 0 R] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0] /C1 [1 1] /N 1 >>", &format!("<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1] /Length {} >>\nstream\n{program}\nendstream",program.len())).encode();
        let EncodedShadingType::RadialAxial(g) = &pattern.shading_type else {
            panic!()
        };
        assert!(pattern.color_space.eval_ops() >= 1000);
        let texture = pattern.render_texture(Rect::new(0., 0., 4096., 1.), 1.);
        assert_eq!(texture.data[0], 128);
        assert_eq!(texture.data[4], 255);
        assert!(pattern.prepared(g).unwrap().runs.get().is_none());
    }

    #[test]
    fn rounded_focal_points_choose_the_continuous_root() {
        for (x, y) in [
            (0.223151, 0.974784),
            (0.28201, 0.959412),
            (0.999999, 0.00148365),
        ] {
            for scale in [0.001, 1., 1000.] {
                let axis = Vec2::new(x * scale, y * scale);
                for t in [0.5, 1.5] {
                    let point = (axis * t + axis.normalize() * (scale * t)).to_point();
                    let forward =
                        radial_pos(point, axis.to_point(), Point::new(0., scale), [false, true])
                            .unwrap();
                    let reverse = radial_pos(
                        point - axis,
                        (-axis).to_point(),
                        Point::new(scale, 0.),
                        [true, false],
                    )
                    .unwrap();
                    assert!((forward - t).abs() < 1e-12);
                    assert!((reverse - (1. - t)).abs() < 1e-12);
                }
            }
        }
        assert_eq!(
            radial_pos(
                Point::new(0.75, 0.),
                Point::new(1.25, 0.),
                Point::new(0., 1.),
                [false, true]
            ),
            Some(3.)
        );
    }

    #[test]
    fn nearly_tangent_extensions_preserve_finite_roots() {
        for (axis, r0, r1, x, expected) in [
            (10000.05, 0., 10000., 0.0605, 0.0605 / 0.05),
            (99.0005, 100., 1., 98.75, -2500.),
            (100.0005, 0., 100., 5.75, 11500.),
            (131073., 0., 131072., 32768., 32768.),
            (131073., 0., 131072., 32769., 32769.),
            (100.0005, 0., 100., 25., 50000.),
            (1. + 2.0_f64.powi(-18), 1., 2., -0.75, 65536.),
            (1. + 2.0_f64.powi(-21), 1., 2., -0.75, 524288.),
            (1.000005, 100000., 99999., 99999.75, -50000.),
        ] {
            let t = radial_pos(
                Point::new(x, 0.),
                Point::new(axis, 0.),
                Point::new(r0, r1),
                [r0 > r1, r1 > r0],
            )
            .unwrap();
            assert!(
                (t - expected).abs() < 1e-6,
                "x={x}, t={t}, expected={expected}"
            );
        }
    }

    #[test]
    fn equal_radii_preserve_extended_root() {
        let axis = Point::new(1., 0.);
        let radii = Point::new(1., 1.);
        assert_eq!(
            radial_pos(Point::new(1.5, 0.), axis, radii, [false, true]),
            Some(2.5)
        );
    }

    #[test]
    fn radial_positions_survive_cancellation() {
        let edge = 500001.0_f64;
        let position = |x| {
            radial_pos(
                Point::new(x, 1.),
                Point::new(0., 1.),
                Point::new(100000., edge),
                [false, false],
            )
        };
        assert_eq!(position(edge), Some(1.));
        assert!(position(edge.next_down()).is_some());
        assert_eq!(position(edge.next_up()), None);
        assert_eq!(
            radial_pos(
                Point::new(0., 0.75),
                Point::new(134217728., 1.),
                Point::new(0., 134217728.),
                [false, false]
            ),
            Some(0.75)
        );
        assert_eq!(
            radial_pos(
                Point::new(-1., 0.),
                Point::new(1., 0.),
                Point::new(1., 2.),
                [false, false]
            ),
            Some(1.)
        );
    }

    #[test]
    fn quarter_turn_axial_texture() {
        let bbox = Rect::new(0., 0., 4., 5.);
        for (coords, expected, horizontal) in [
            ("0 0 8 0", Rect::new(0., 0., 1., 5.), true),
            ("0 0 0 -8", Rect::new(0., 0., 4., 1.), false),
        ] {
            let source = format!(
                "<< /ShadingType 2 /Coords [{coords}] /ColorSpace /DeviceGray
                /Function << /FunctionType 2 /Domain [0 1] /N 1 >> >>"
            );
            let dict = Dict::from_bytes(source.as_bytes()).unwrap();
            let pattern = ShadingPattern {
                shading: Arc::new(Shading::new(&dict, None, &Cache::default()).unwrap()),
                matrix: Affine::new([0., 1., -1., 0., 0., 0.]),
                opacity: 1.,
                transfer_function: None,
            }
            .encode();
            let EncodedShadingType::RadialAxial(g) = &pattern.shading_type else {
                unreachable!()
            };
            assert_eq!(g.texture_bounds(bbox, &pattern), expected);
            let mut pixels = Vec::new();
            pattern.sample_texture(bbox, 1., |c| pixels.push(c));
            for y in 0..5 {
                for x in 0..4 {
                    let t = (if horizontal { y } else { x } as f64 + 0.5) / 8.;
                    assert_eq!(pixels[y * 4 + x], pattern.eval_1d(&g.function, t));
                }
            }
        }
    }

    #[test]
    fn axial_base_transform_normalizes_endpoints() {
        let dict = Dict::from_bytes(b"<< /ShadingType 2 /Coords [4 2 8 2] /ColorSpace /DeviceGray /Function << /FunctionType 2 /Domain [0 1] /N 1 >> >>").unwrap();
        let pattern = ShadingPattern {
            shading: Arc::new(Shading::new(&dict, None, &Cache::default()).unwrap()),
            matrix: Affine::IDENTITY,
            opacity: 1.0,
            transfer_function: None,
        }
        .encode();
        assert_eq!(pattern.base_transform * Point::new(4.0, 2.0), Point::ZERO);
        assert_eq!(
            pattern.base_transform * Point::new(8.0, 2.0),
            Point::new(1.0, 0.0)
        );
    }
}
