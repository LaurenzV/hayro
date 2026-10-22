//! Encoding shading patterns for easy sampling.

use crate::color::{AlphaColor, ColorSpace};
use crate::interpret::state::ActiveTransferFunction;
use crate::pattern::ShadingPattern;
use crate::shading::{ShadingFunction, ShadingType, Triangle};
use crate::util::x_y_advances;
use kurbo::{Affine, Point, Rect};
use smallvec::{ToSmallVec, smallvec};
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
    pub(crate) triangles: Arc<[PlacedTriangle]>,
    /// The pixels of the boxes of all triangles.
    pub(crate) bounds: PixelRect,
    pub(crate) function: Option<ShadingFunction>,
}

/// A triangle of a mesh in device space, with the pixels of its box.
///
/// The color of a pixel `(x, y)` is that of the last triangle whose box holds it
/// and that contains the point `(x, y)`, interpolated there.
#[derive(Clone, Debug)]
pub(crate) struct PlacedTriangle {
    triangle: Triangle,
    pixels: PixelRect,
}

/// A rectangle of device pixels.
#[derive(Clone, Debug, Default)]
pub(crate) struct PixelRect {
    xs: Range<u16>,
    ys: Range<u16>,
}

impl PixelRect {
    fn contains(&self, x: u16, y: u16) -> bool {
        self.xs.contains(&x) && self.ys.contains(&y)
    }

    fn intersect(&self, other: &Self) -> Self {
        Self {
            xs: self.xs.start.max(other.xs.start)..self.xs.end.min(other.xs.end),
            ys: self.ys.start.max(other.ys.start)..self.ys.end.min(other.ys.end),
        }
    }
}

/// For each pixel of a rectangle, the triangle that colors it.
pub(crate) struct Coverage {
    rect: PixelRect,
    /// The index of the triangle plus one, or 0 where none covers the pixel.
    owners: Vec<u32>,
}

impl Coverage {
    /// The index of the triangle that colors the pixel `(x, y)`, if the pixel lies
    /// in the rectangle.
    fn owner(&self, x: u16, y: u16) -> Option<Option<usize>> {
        if !self.rect.contains(x, y) {
            return None;
        }

        let index = usize::from(y - self.rect.ys.start) * self.rect.xs.len()
            + usize::from(x - self.rect.xs.start);
        Some(self.owners[index].checked_sub(1).map(|i| i as usize))
    }
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
    ///
    /// For a mesh shading, this searches the triangles of the mesh; use
    /// [`sample_texture`](Self::sample_texture) to sample many positions.
    #[inline]
    pub fn sample(&self, pos: Point) -> [f32; 4] {
        self.sample_with(pos, None)
    }

    #[inline]
    fn sample_with(&self, pos: Point, coverage: Option<&Coverage>) -> [f32; 4] {
        self.shading_type
            .eval(pos, self.background_color, &self.color_space, coverage)
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

        // A mesh is sampled only at the pixels the points below fall in.
        let coverage = match &self.shading_type {
            EncodedShadingType::Sampled(s) => {
                let read = self.base_transform.transform_rect_bbox(Rect::new(
                    bbox.x0 + 0.5 * inv_scale,
                    bbox.y0 + 0.5 * inv_scale,
                    bbox.x0 + (width as f64 - 0.5) * inv_scale,
                    bbox.y0 + (height as f64 - 0.5) * inv_scale,
                ));
                Some(s.coverage(&read))
            }
            _ => None,
        };

        for _ in 0..height {
            let mut point = start_point;

            for _ in 0..width {
                f(self.sample_with(point, coverage.as_ref()));

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
                base_transform = Affine::IDENTITY;

                encode_mesh(triangles, self.matrix, function)
            }
            ShadingType::CoonsPatchMesh { patches, function } => {
                let mut triangles = vec![];
                for patch in patches {
                    patch.to_triangles(&mut triangles);
                }

                base_transform = Affine::IDENTITY;

                encode_mesh(&triangles, self.matrix, function)
            }
            ShadingType::TensorProductPatchMesh { patches, function } => {
                let mut triangles = vec![];
                for patch in patches {
                    patch.to_triangles(&mut triangles);
                }

                base_transform = Affine::IDENTITY;

                encode_mesh(&triangles, self.matrix, function)
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

fn encode_mesh(
    triangles: &[Triangle],
    transform: Affine,
    function: &Option<ShadingFunction>,
) -> EncodedShadingType {
    let mut bounds: Option<PixelRect> = None;

    let triangles: Arc<[PlacedTriangle]> = triangles
        .iter()
        .map(|t| {
            let mut v0 = t.p0.clone();
            v0.point = transform * t.p0.point;
            let mut v1 = t.p1.clone();
            v1.point = transform * t.p1.point;
            let mut v2 = t.p2.clone();
            v2.point = transform * t.p2.point;
            let triangle = Triangle::new(v0, v1, v2);

            let bbox = triangle.bounding_box();
            let pixels = PixelRect {
                xs: (bbox.x0.floor() as u16)..(bbox.x1.ceil() as u16),
                ys: (bbox.y0.floor() as u16)..(bbox.y1.ceil() as u16),
            };

            if !pixels.xs.is_empty() && !pixels.ys.is_empty() {
                bounds = Some(match bounds.take() {
                    None => pixels.clone(),
                    Some(b) => PixelRect {
                        xs: b.xs.start.min(pixels.xs.start)..b.xs.end.max(pixels.xs.end),
                        ys: b.ys.start.min(pixels.ys.start)..b.ys.end.max(pixels.ys.end),
                    },
                });
            }

            PlacedTriangle { triangle, pixels }
        })
        .collect();

    EncodedShadingType::Sampled(EncodedSampledShading {
        triangles,
        bounds: bounds.unwrap_or_default(),
        function: function.clone(),
    })
}

impl EncodedSampledShading {
    /// The triangle that colors the pixel `(x, y)`, looked up in `coverage` where
    /// it holds the pixel, else searched for among all.
    fn triangle_at(&self, x: u16, y: u16, coverage: Option<&Coverage>) -> Option<&Triangle> {
        if let Some(owner) = coverage.and_then(|c| c.owner(x, y)) {
            return owner.map(|i| &self.triangles[i].triangle);
        }

        if !self.bounds.contains(x, y) {
            return None;
        }

        let point = Point::new(x as f64, y as f64);
        self.triangles
            .iter()
            .rev()
            .find(|t| t.pixels.contains(x, y) && t.triangle.contains_point(point))
            .map(|t| &t.triangle)
    }

    /// Find the triangle that colors each pixel of `read`, a rectangle in device
    /// space that holds the positions a texture samples.
    fn coverage(&self, read: &Rect) -> Coverage {
        // Where rounding moves a position read into a pixel outside the
        // rectangle, `triangle_at` still finds its triangle, only slower.
        let read = PixelRect {
            xs: (read.x0 as u16)..(read.x1 as u16).saturating_add(1),
            ys: (read.y0 as u16)..(read.y1 as u16).saturating_add(1),
        };
        let rect = self.bounds.intersect(&read);
        let width = rect.xs.len();
        let mut owners = vec![0; width * rect.ys.len()];

        for (t, owner) in self.triangles.iter().zip(1_u32..) {
            let pixels = t.pixels.intersect(&rect);

            for y in pixels.ys {
                for x in pixels.xs.clone() {
                    if t.triangle.contains_point(Point::new(x as f64, y as f64)) {
                        let index =
                            usize::from(y - rect.ys.start) * width + usize::from(x - rect.xs.start);
                        owners[index] = owner;
                    }
                }
            }
        }

        Coverage { rect, owners }
    }
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
        coverage: Option<&Coverage>,
    ) -> Option<AlphaColor> {
        match self {
            Self::FunctionBased(EncodedFunctionBasedShading { domain, function }) => {
                if !domain.contains(pos) {
                    Some(bg_color)
                } else {
                    let out = function.eval(&smallvec![pos.x as f32, pos.y as f32])?;
                    // TODO: Clamp out-of-range values.
                    Some(color_space.to_rgba(&out, 1.0))
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

                let val = function.eval(&smallvec![t])?;

                Some(color_space.to_rgba(&val, 1.0))
            }
            Self::Sampled(shading) => {
                let (x, y) = (pos.x as u16, pos.y as u16);

                if let Some(triangle) = shading.triangle_at(x, y, coverage) {
                    let color = triangle.interpolate(Point::new(x as f64, y as f64));

                    if let Some(function) = &shading.function {
                        let val = function.eval(&color.to_smallvec())?;
                        Some(color_space.to_rgba(&val, 1.0))
                    } else {
                        Some(color_space.to_rgba(&color, 1.0))
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

#[cfg(test)]
mod tests {
    use super::{EncodedShadingType, encode_mesh};
    use crate::color::ColorComponents;
    use crate::shading::{Triangle, TriangleVertex};
    use kurbo::{Affine, Point, Rect};
    use rustc_hash::FxHashMap;
    use smallvec::smallvec;

    struct Random(u64);

    impl Random {
        fn below(&mut self, n: f64) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % 1_000_000) as f64 / 1_000_000.0 * n
        }
    }

    fn triangle(points: [(f64, f64); 3], color: f32) -> Triangle {
        let [a, b, c] = points.map(|(x, y)| {
            TriangleVertex::at(Point::new(x, y), smallvec![color, color / 2.0, 1.0 - color])
        });
        Triangle::new(a, b, c)
    }

    /// Check that the colors of a mesh, looked up in its coverage of `read` and
    /// searched for without it, are those of putting each covered pixel of each
    /// triangle into a map, the later over the earlier, at every pixel of the
    /// window `0..400` by `0..400`, and return how many pixels the coverage holds.
    fn check(triangles: &[Triangle], transform: Affine, read: Rect) -> usize {
        let mut expected = FxHashMap::<(u16, u16), ColorComponents>::default();
        for t in triangles {
            let [p0, p1, p2] = [&t.p0, &t.p1, &t.p2].map(|v| {
                let mut v = (*v).clone();
                v.point = transform * v.point;
                v
            });
            let t = Triangle::new(p0, p1, p2);
            let bbox = t.bounding_box();
            for y in (bbox.y0.floor() as u16)..(bbox.y1.ceil() as u16).min(400) {
                for x in (bbox.x0.floor() as u16)..(bbox.x1.ceil() as u16).min(400) {
                    let point = Point::new(x as f64, y as f64);
                    if t.contains_point(point) {
                        expected.insert((x, y), t.interpolate(point));
                    }
                }
            }
        }

        let EncodedShadingType::Sampled(mesh) = encode_mesh(triangles, transform, &None) else {
            unreachable!()
        };
        let coverage = mesh.coverage(&read);
        for y in 0..400 {
            for x in 0..400 {
                let point = Point::new(x as f64, y as f64);
                for found in [
                    mesh.triangle_at(x, y, Some(&coverage)),
                    mesh.triangle_at(x, y, None),
                ] {
                    assert_eq!(
                        found.map(|t| t.interpolate(point)).as_ref(),
                        expected.get(&(x, y)),
                        "({x}, {y})"
                    );
                }
            }
        }

        coverage.owners.len()
    }

    #[test]
    fn a_mesh_is_sampled_only_in_the_pixels_read() {
        // A diagonal sliver whose box grows with it, read in 400 by 300 pixels.
        let read = Rect::new(50.5, 20.5, 449.5, 319.5);
        for size in [100.0, 1000.0, 10000.0, 65534.0] {
            let sliver = [triangle(
                [(0.0, 0.0), (size, size), (size, size - 0.01)],
                0.5,
            )];
            let cells = check(&sliver, Affine::IDENTITY, read);
            assert!(cells <= 400 * 300, "{size}: {cells}");
        }
    }

    #[test]
    fn a_mesh_is_sampled_as_a_map_of_its_covered_pixels_would_hold_it() {
        let mut random = Random(0x2545_f491_4f6c_dd1d);
        // A tiling of a box, with triangles over it that cover others.
        let mut tiles = vec![];
        for row in 0..6 {
            for column in 0..6 {
                let (x, y) = (20.0 + column as f64 * 30.0, 40.0 + row as f64 * 25.0);
                let color = random.below(1.0) as f32;
                tiles.push(triangle([(x, y), (x + 30.0, y), (x, y + 25.0)], color));
                tiles.push(triangle(
                    [(x + 30.0, y), (x + 30.0, y + 25.0), (x, y + 25.0)],
                    color / 3.0,
                ));
            }
        }
        for _ in 0..10 {
            let corner =
                |random: &mut Random| (20.0 + random.below(180.0), 40.0 + random.below(150.0));
            let points = [
                corner(&mut random),
                corner(&mut random),
                corner(&mut random),
            ];
            tiles.push(triangle(points, random.below(1.0) as f32));
        }

        for transform in [
            Affine::IDENTITY,
            Affine::new([1.5, 0.3, -0.2, 1.1, 10.0, -12.5]),
            // Off the device.
            Affine::translate((-500.0, -500.0)),
        ] {
            for read in [
                Rect::new(0.5, 0.5, 399.5, 399.5),
                Rect::new(60.5, 90.5, 129.5, 139.5),
                Rect::new(-10.0, -10.0, -5.0, -5.0),
            ] {
                check(&tiles, transform, read);
            }
        }
        // Slivers far apart.
        let slivers = [
            triangle([(1.0, 1.0), (3.0, 1.0), (1.0, 390.0)], 0.25),
            triangle([(380.0, 1.0), (383.0, 1.0), (380.0, 390.0)], 0.75),
        ];
        check(
            &slivers,
            Affine::IDENTITY,
            Rect::new(0.5, 0.5, 399.5, 399.5),
        );
        check(&[], Affine::IDENTITY, Rect::new(0.5, 0.5, 399.5, 399.5));
    }
}
