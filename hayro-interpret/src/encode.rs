//! Encoding shading patterns for easy sampling.

use crate::color::{AlphaColor, ColorComponents, ColorSpace};
use crate::function::MAX_COMPONENTS;
use crate::interpret::state::ActiveTransferFunction;
use crate::pattern::ShadingPattern;
use crate::shading::{ShadingFunction, ShadingType, Triangle};
use crate::util::x_y_advances;
use kurbo::{Affine, Point, Rect, Shape};
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::sync::Arc;

/// A shading pattern that was encoded so it can be sampled.
#[derive(Clone, Debug)]
pub struct EncodedShadingPattern {
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
    /// Mesh-based shading sampled into device pixels.
    Sampled(EncodedSampledShading),
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

/// Encoded sampled mesh shading.
#[derive(Clone, Debug)]
pub struct EncodedSampledShading {
    pub(crate) samples: Arc<MeshSamples>,
    pub(crate) function: Option<ShadingFunction>,
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
    /// The transform from texture space to the space of the rasterized
    /// bounding box.
    pub transform: Affine,
    /// Whether any pixel of the texture is not fully opaque.
    pub has_transparency: bool,
}

/// Return the texture dimensions [`EncodedShadingPattern::sample_texture`]
/// uses for the given bounding box and scale factor.
///
/// Exposed so that callers can bound the size of the resulting texture
/// before sampling.
pub fn texture_dimensions(bbox: Rect, scale: f32) -> (u32, u32) {
    (
        (bbox.width() as f32 * scale).max(1.0).ceil() as u32,
        (bbox.height() as f32 * scale).max(1.0).ceil() as u32,
    )
}

impl EncodedShadingPattern {
    /// Sample the shading at the given position.
    #[inline]
    pub fn sample(&self, pos: Point) -> [f32; 4] {
        self.shading_type
            .eval(pos, self.background_color, &self.color_space)
            .map(|v| {
                let mut components = v.components();
                components[3] *= self.opacity;

                if let Some(tf) = &self.transfer_function {
                    return tf.apply(&AlphaColor::new(components)).components();
                }

                components
            })
            .unwrap_or([0.0, 0.0, 0.0, 0.0])
    }

    /// Sample the shading on a pixel grid covering `bbox`, with `scale`
    /// pixels per unit of `bbox`.
    ///
    /// The callback is invoked once per pixel in row-major order with the
    /// straight (non-premultiplied) RGBA sample, letting callers apply their
    /// own conversion. Returns the dimensions of the grid (see
    /// [`texture_dimensions`]) and the transform from grid space to the space
    /// of `bbox`. No size cap is applied — callers that need to bound the
    /// texture size should check [`texture_dimensions`] first.
    pub fn sample_texture(
        &self,
        bbox: Rect,
        scale: f32,
        mut f: impl FnMut([f32; 4]),
    ) -> (u32, u32, Affine) {
        let (width, height) = texture_dimensions(bbox, scale);
        let inv_scale = 1.0 / scale as f64;

        let (x_advance, y_advance) =
            x_y_advances(&(Affine::scale(inv_scale) * self.base_transform));

        let mut start_point = self.base_transform
            * Affine::translate((0.5 * inv_scale, 0.5 * inv_scale))
            * Point::new(bbox.x0, bbox.y0);

        for _ in 0..height {
            let mut point = start_point;

            for _ in 0..width {
                f(self.sample(point));

                point += x_advance;
            }

            start_point += y_advance;
        }

        (
            width,
            height,
            Affine::translate((bbox.x0, bbox.y0)) * Affine::scale(inv_scale),
        )
    }

    /// Rasterize the shading into a straight (non-premultiplied) RGBA8
    /// texture covering `bbox`, with `scale` pixels per unit of `bbox`.
    ///
    /// See [`sample_texture`](Self::sample_texture) for the grid semantics.
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
        self.encode_within(Rect::new(0.0, 0.0, 65536.0, 65536.0))
    }

    /// Encode the shading pattern for drawing within `target`, in device pixels. Mesh
    /// shadings, which are sampled at device pixels, are only sampled within it.
    pub fn encode_within(&self, target: Rect) -> EncodedShadingPattern {
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
                let (encoded, initial_transform) =
                    encode_axial_shading(*coords, *domain, function, *extend, *axial);

                base_transform = initial_transform * self.matrix.inverse();

                encoded
            }
            ShadingType::TriangleMesh {
                triangles,
                function,
            } => {
                let full_transform = self.matrix;
                let samples = sample_triangles(triangles, full_transform, target);

                base_transform = Affine::IDENTITY;

                EncodedShadingType::Sampled(EncodedSampledShading {
                    samples: Arc::new(samples),
                    function: function.clone(),
                })
            }
            ShadingType::CoonsPatchMesh { patches, function } => {
                let mut triangles = vec![];
                for patch in patches {
                    patch.to_triangles(&mut triangles);
                }

                let full_transform = self.matrix;
                let samples = sample_triangles(&triangles, full_transform, target);

                base_transform = Affine::IDENTITY;

                EncodedShadingType::Sampled(EncodedSampledShading {
                    samples: Arc::new(samples),
                    function: function.clone(),
                })
            }
            ShadingType::TensorProductPatchMesh { patches, function } => {
                let mut triangles = vec![];
                for patch in patches {
                    patch.to_triangles(&mut triangles);
                }

                let full_transform = self.matrix;
                let samples = sample_triangles(&triangles, full_transform, target);

                base_transform = Affine::IDENTITY;

                EncodedShadingType::Sampled(EncodedSampledShading {
                    samples: Arc::new(samples),
                    function: function.clone(),
                })
            }
            ShadingType::Dummy => {
                base_transform = Affine::IDENTITY;

                EncodedShadingType::Dummy(EncodedDummyShading)
            }
        };

        let color_space = self.shading.color_space.clone();

        let background_color = self
            .shading
            .background
            .as_ref()
            .map(|b| color_space.to_rgba(b, 1.0))
            .unwrap_or(AlphaColor::TRANSPARENT);

        EncodedShadingPattern {
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
    coords: [f32; 6],
    domain: [f32; 2],
    function: &ShadingFunction,
    extend: [bool; 2],
    is_axial: bool,
) -> (EncodedShadingType, Affine) {
    let initial_transform;

    let params = if is_axial {
        let [x_0, y_0, x_1, y_1, _, _] = coords;

        initial_transform = ts_from_line_to_line(
            Point::new(x_0 as f64, y_0 as f64),
            Point::new(x_1 as f64, y_1 as f64),
            Point::ZERO,
            Point::new(1.0, 0.0),
        );

        RadialAxialParams::Axial
    } else {
        let [x_0, y_0, r0, x_1, y_1, r_1] = coords;

        initial_transform = Affine::translate((-x_0 as f64, -y_0 as f64));
        let new_x1 = x_1 - x_0;
        let new_y1 = y_1 - y_0;

        let p1 = Point::new(new_x1 as f64, new_y1 as f64);
        let r = Point::new(r0 as f64, r_1 as f64);

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

/// Mesh shadings whose triangles span at most this many device pixels are sampled into a
/// grid; larger ones keep only the pixels their triangles cover.
const MAX_DENSE_MESH_PIXELS: usize = 1 << 25;

/// The colors of a mesh shading at the device pixels its triangles cover.
#[derive(Debug)]
pub(crate) enum MeshSamples {
    Dense {
        x0: u16,
        y0: u16,
        width: usize,
        height: usize,
        /// The number of components of each pixel.
        stride: usize,
        components: Vec<f32>,
        covered: Vec<bool>,
    },
    Sparse(FxHashMap<(u16, u16), ColorComponents>),
}

impl MeshSamples {
    #[inline]
    fn get(&self, (x, y): (u16, u16)) -> Option<&[f32]> {
        match self {
            Self::Dense {
                x0,
                y0,
                width,
                height,
                stride,
                components,
                covered,
            } => {
                let x = x.checked_sub(*x0)? as usize;
                let y = y.checked_sub(*y0)? as usize;
                if x >= *width || y >= *height {
                    return None;
                }

                let i = y * width + x;
                covered[i].then(|| &components[i * stride..][..*stride])
            }
            Self::Sparse(map) => map.get(&(x, y)).map(|c| c.as_slice()),
        }
    }
}

/// The device pixels a triangle's bounding box covers, as in [`sample_triangles`].
fn pixel_span(bbox: Rect) -> (Range<u16>, Range<u16>) {
    (
        (bbox.x0.floor() as u16)..(bbox.x1.ceil() as u16),
        (bbox.y0.floor() as u16)..(bbox.y1.ceil() as u16),
    )
}

/// The part of a span of pixels within another.
fn clip(span: Range<u16>, within: &Range<u16>) -> Range<u16> {
    span.start.max(within.start)..span.end.min(within.end)
}

/// Call `f` with each pixel within `xs` by `ys` that the triangle, mapped by `transform`,
/// covers, and the weights of its three vertices' colors there.
fn for_each_pixel(
    t: &Triangle,
    transform: Affine,
    xs: &Range<u16>,
    ys: &Range<u16>,
    mut f: impl FnMut(u16, u16, [f32; 3]),
) {
    let [a, b, c] = [t.p0.point, t.p1.point, t.p2.point].map(|p| transform * p);
    let triangle = kurbo::Triangle::new(a, b, c);
    let (t_xs, t_ys) = pixel_span(triangle.bounding_box());
    let (t_xs, t_ys) = (clip(t_xs, xs), clip(t_ys, ys));
    if t_xs.is_empty() || t_ys.is_empty() {
        return;
    }

    // As in `Triangle::interpolate`.
    let (v0, v1) = (b - a, c - a);
    let (d00, d01, d11) = (v0.dot(v0), v0.dot(v1), v1.dot(v1));
    let denom = d00 * d11 - d01 * d01;

    for y in t_ys {
        for x in t_xs.clone() {
            let point = Point::new(x as f64, y as f64);
            if triangle.winding(point) != 0 {
                let v2 = point - a;
                let (d20, d21) = (v2.dot(v0), v2.dot(v1));
                let v = (d11 * d20 - d01 * d21) / denom;
                let w = (d00 * d21 - d01 * d20) / denom;
                f(x, y, [(1.0 - v - w) as f32, v as f32, w as f32]);
            }
        }
    }
}

/// The color at a point of a triangle, from its vertices' weights there.
#[inline]
fn interpolate(t: &Triangle, [u, v, w]: [f32; 3], out: &mut [f32]) {
    for (i, out) in out.iter_mut().enumerate() {
        *out = u * t.p0.colors[i] + v * t.p1.colors[i] + w * t.p2.colors[i];
    }
}

fn sample_triangles(triangles: &[Triangle], transform: Affine, target: Rect) -> MeshSamples {
    // The transformed bounds of all the vertices contain every triangle's pixels.
    let stride = triangles.first().map_or(0, |t| t.p0.colors.len());
    let mut same_stride = true;
    let (mut min, mut max) = (
        Point::new(f64::MAX, f64::MAX),
        Point::new(f64::MIN, f64::MIN),
    );
    for vertex in triangles.iter().flat_map(|t| [&t.p0, &t.p1, &t.p2]) {
        let p = vertex.point;
        (min.x, min.y, max.x, max.y) = (
            min.x.min(p.x),
            min.y.min(p.y),
            max.x.max(p.x),
            max.y.max(p.y),
        );
        same_stride &= vertex.colors.len() == stride;
    }
    let bounds = transform.transform_rect_bbox(Rect::from_points(min, max));
    let (target_xs, target_ys) = pixel_span(target);
    let (xs, ys) = pixel_span(bounds);
    let (xs, ys) = (clip(xs, &target_xs), clip(ys, &target_ys));
    let (x0, y0) = (xs.start, ys.start);
    let (width, height) = (xs.len(), ys.len());

    if !same_stride || width * height > MAX_DENSE_MESH_PIXELS {
        return MeshSamples::Sparse(sample_triangles_sparse(triangles, transform, target));
    }

    let mut components = vec![0.0; width * height * stride];
    let mut covered = vec![false; width * height];

    for t in triangles {
        for_each_pixel(t, transform, &xs, &ys, |x, y, weights| {
            let i = (y - y0) as usize * width + (x - x0) as usize;
            interpolate(t, weights, &mut components[i * stride..][..stride]);
            covered[i] = true;
        });
    }

    MeshSamples::Dense {
        x0,
        y0,
        width,
        height,
        stride,
        components,
        covered,
    }
}

fn sample_triangles_sparse(
    triangles: &[Triangle],
    transform: Affine,
    target: Rect,
) -> FxHashMap<(u16, u16), ColorComponents> {
    let mut map = FxHashMap::default();
    let (xs, ys) = pixel_span(target);

    for t in triangles {
        for_each_pixel(t, transform, &xs, &ys, |x, y, weights| {
            let mut color = ColorComponents::from_elem(0.0, t.p0.colors.len());
            interpolate(t, weights, &mut color);
            map.insert((x, y), color);
        });
    }

    map
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
    Axial,
    Radial { p1: Point, r: Point },
}

impl EncodedShadingType {
    pub(crate) fn eval(
        &self,
        pos: Point,
        bg_color: AlphaColor,
        color_space: &ColorSpace,
    ) -> Option<AlphaColor> {
        match self {
            Self::FunctionBased(EncodedFunctionBasedShading { domain, function }) => {
                if !domain.contains(pos) {
                    Some(bg_color)
                } else {
                    let mut out = [0.0; MAX_COMPONENTS];
                    let out = &mut out[..function.num_outputs()];
                    function.eval(&[pos.x as f32, pos.y as f32], out)?;
                    // TODO: Clamp out-of-range values.
                    Some(color_space.to_rgba(out, 1.0))
                }
            }
            Self::RadialAxial(EncodedRadialAxialShading {
                function,
                params,
                domain,
                extend,
            }) => {
                let (t0, t1) = (domain[0], domain[1]);

                let mut t = match params {
                    RadialAxialParams::Axial => pos.x as f32,
                    RadialAxialParams::Radial { p1, r } => {
                        radial_pos(&pos, p1, *r, extend[0], extend[1]).unwrap_or(f32::MIN)
                    }
                };

                if t == f32::MIN {
                    return Some(bg_color);
                }

                if t < 0.0 {
                    if extend[0] {
                        t = 0.0;
                    } else {
                        return Some(bg_color);
                    }
                } else if t > 1.0 {
                    if extend[1] {
                        t = 1.0;
                    } else {
                        return Some(bg_color);
                    }
                }

                let t = t0 + (t1 - t0) * t;

                let mut val = [0.0; MAX_COMPONENTS];
                let val = &mut val[..function.num_outputs()];
                function.eval(&[t], val)?;

                Some(color_space.to_rgba(val, 1.0))
            }
            Self::Sampled(EncodedSampledShading { samples, function }) => {
                let sample_point = (pos.x as u16, pos.y as u16);

                if let Some(color) = samples.get(sample_point) {
                    if let Some(function) = function {
                        let mut val = [0.0; MAX_COMPONENTS];
                        let val = &mut val[..function.num_outputs()];
                        function.eval(color, val)?;
                        Some(color_space.to_rgba(val, 1.0))
                    } else {
                        Some(color_space.to_rgba(color, 1.0))
                    }
                } else {
                    Some(bg_color)
                }
            }
            Self::Dummy(_) => Some(AlphaColor::TRANSPARENT),
        }
    }
}

fn ts_from_line_to_line(src1: Point, src2: Point, dst1: Point, dst2: Point) -> Affine {
    let unit_to_line1 = unit_to_line(src1, src2);
    let line1_to_unit = unit_to_line1.inverse();
    let unit_to_line2 = unit_to_line(dst1, dst2);

    unit_to_line2 * line1_to_unit
}

fn unit_to_line(p0: Point, p1: Point) -> Affine {
    Affine::new([
        p1.y - p0.y,
        p0.x - p1.x,
        p1.x - p0.x,
        p1.y - p0.y,
        p0.x,
        p0.y,
    ])
}

fn radial_pos(
    pos: &Point,
    p1: &Point,
    r: Point,
    min_extend: bool,
    max_extend: bool,
) -> Option<f32> {
    let r0 = r.x as f32;
    let dx = p1.x as f32;
    let dy = p1.y as f32;
    let dr = r.y as f32 - r0;

    let px = pos.x as f32;
    let py = pos.y as f32;

    let a = dx * dx + dy * dy - dr * dr;
    let b = -2.0 * (px * dx + py * dy + r0 * dr);
    let c = px * px + py * py - r0 * r0;

    let discriminant = b * b - 4.0 * a * c;

    // No solution available.
    if discriminant < 0.0 {
        return None;
    }

    if a.abs() < 1e-6 {
        if b.abs() < 1e-6 {
            return None;
        }

        let t = -c / b;

        if (!min_extend && t < 0.0) || (!max_extend && t > 1.0) {
            return None;
        }

        let r_t = r0 + dr * t;
        if r_t < 0.0 {
            return None;
        }

        return Some(t);
    }

    let sqrt_d = discriminant.sqrt();
    let t1 = (-b - sqrt_d) / (2.0 * a);
    let t2 = (-b + sqrt_d) / (2.0 * a);

    let max = t1.max(t2);
    let mut take_max = Some(max);
    let min = t1.min(t2);
    let mut take_min = Some(min);

    if (!min_extend && min < 0.0) || r0 + dr * min < 0.0 {
        take_min = None;
    }

    if (!max_extend && max > 1.0) || r0 + dr * max < 0.0 {
        take_max = None;
    }

    match (take_min, take_max) {
        (Some(_), Some(max)) => Some(max),
        (Some(min), None) => Some(min),
        (None, Some(max)) => Some(max),
        (None, None) => None,
    }
}
