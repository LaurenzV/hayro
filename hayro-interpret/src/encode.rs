//! Encoding shading patterns for easy sampling.

use crate::color::{AlphaColor, ColorSpace};
use crate::interpret::state::ActiveTransferFunction;
use crate::pattern::ShadingPattern;
use crate::shading::{ShadingFunction, ShadingType, Triangle};
use crate::util::x_y_advances;
use kurbo::{Affine, Point, Rect, Vec2};
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
#[derive(Clone, Debug)]
pub(crate) struct PixelRect {
    xs: Range<u16>,
    ys: Range<u16>,
}

impl PixelRect {
    fn contains(&self, (x, y): (u16, u16)) -> bool {
        self.xs.contains(&x) && self.ys.contains(&y)
    }

    fn intersect(&self, other: &Self) -> Self {
        Self {
            xs: self.xs.start.max(other.xs.start)..self.xs.end.min(other.xs.end),
            ys: self.ys.start.max(other.ys.start)..self.ys.end.min(other.ys.end),
        }
    }

    fn is_empty(&self) -> bool {
        self.xs.is_empty() || self.ys.is_empty()
    }
}

/// The pixel a position falls in.
fn pixel(pos: Point) -> (u16, u16) {
    (pos.x as u16, pos.y as u16)
}

/// For each position a texture samples, in the order it samples them, the pixel
/// the position falls in and the triangle that colors that pixel.
pub(crate) struct Coverage {
    pixels: Vec<(u16, u16)>,
    /// The index of the triangle plus one, or 0 where none covers the pixel.
    owners: Vec<u32>,
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
        self.finish(
            self.shading_type
                .eval(pos, self.background_color, &self.color_space),
        )
    }

    /// Apply the opacity and the transfer function to a color of the shading.
    #[inline]
    fn finish(&self, color: Option<AlphaColor>) -> [f32; 4] {
        color
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
    /// texture size should check [`texture_dimensions`] first. For a mesh
    /// shading, this keeps 8 bytes per pixel of the grid while sampling.
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

        if let EncodedShadingType::Sampled(mesh) = &self.shading_type {
            // A mesh is colored only at the pixels the positions fall in.
            let coverage = mesh.coverage(
                start_point,
                x_advance,
                y_advance,
                width as usize,
                height as usize,
            );

            for (&(x, y), &owner) in coverage.pixels.iter().zip(&coverage.owners) {
                let triangle = owner
                    .checked_sub(1)
                    .map(|i| &mesh.triangles[i as usize].triangle);
                f(self.finish(mesh.color(
                    triangle,
                    (x, y),
                    self.background_color,
                    &self.color_space,
                )));
            }
        } else {
            for _ in 0..height {
                let mut point = start_point;

                for _ in 0..width {
                    f(self.sample(point));

                    point += x_advance;
                }

                start_point += y_advance;
            }
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

            PlacedTriangle { triangle, pixels }
        })
        .collect();

    EncodedShadingType::Sampled(EncodedSampledShading {
        triangles,
        function: function.clone(),
    })
}

impl EncodedSampledShading {
    /// The triangle that colors the pixel `(x, y)`, searched for among all.
    fn triangle_at(&self, (x, y): (u16, u16)) -> Option<&Triangle> {
        let point = Point::new(x as f64, y as f64);
        self.triangles
            .iter()
            .rev()
            .find(|t| t.pixels.contains((x, y)) && t.triangle.contains_point(point))
            .map(|t| &t.triangle)
    }

    /// The color of the pixel `(x, y)`, which `triangle` colors.
    fn color(
        &self,
        triangle: Option<&Triangle>,
        (x, y): (u16, u16),
        bg_color: AlphaColor,
        color_space: &ColorSpace,
    ) -> Option<AlphaColor> {
        let Some(triangle) = triangle else {
            return Some(bg_color);
        };

        let color = triangle.interpolate(Point::new(x as f64, y as f64));

        if let Some(function) = &self.function {
            let val = function.eval(&color.to_smallvec())?;
            Some(color_space.to_rgba(&val, 1.0))
        } else {
            Some(color_space.to_rgba(&color, 1.0))
        }
    }

    /// Find the pixels of `height` rows of `width` positions, the first at
    /// `start`, each next one in a row `x_advance` further and each next row
    /// `y_advance` further, and the triangle that colors each pixel.
    fn coverage(
        &self,
        start: Point,
        x_advance: Vec2,
        y_advance: Vec2,
        width: usize,
        height: usize,
    ) -> Coverage {
        if width == 0 || height == 0 {
            return Coverage {
                pixels: vec![],
                owners: vec![],
            };
        }

        let mut pixels = Vec::with_capacity(width * height);
        let (mut low, mut high) = ((u16::MAX, u16::MAX), (0, 0));
        let mut row_start = start;
        for _ in 0..height {
            let mut point = row_start;
            for _ in 0..width {
                let (x, y) = pixel(point);
                pixels.push((x, y));
                low = (low.0.min(x), low.1.min(y));
                high = (high.0.max(x), high.1.max(y));
                point += x_advance;
            }
            row_start += y_advance;
        }
        // The pixels from the least x and y of these pixels to the greatest.
        let reached = PixelRect {
            xs: low.0..high.0.saturating_add(1),
            ys: low.1..high.1.saturating_add(1),
        };

        let mut owners = vec![0; pixels.len()];

        // Each position adds the same advance to the one before it in its row
        // or column, and rounding keeps order, so along a row each coordinate
        // of the pixels only grows or only shrinks, as the sign of the advance
        // says, and likewise down a column. The positions of a row that fall in
        // a rectangle of pixels are thus a run, and so are the rows whose pixels
        // span it.
        let along_row = (x_advance.x >= 0.0, x_advance.y >= 0.0);
        let down_column = (y_advance.x >= 0.0, y_advance.y >= 0.0);
        // With upright advances, as a mesh has unless its base transform is
        // changed, every row has the x of the first row and a single y.
        let upright = x_advance.y == 0.0 && y_advance.x == 0.0;
        let row = |j: usize| &pixels[j * width..][..width];
        // The least and the greatest x and y of the pixels of each row.
        let spans: Vec<_> = (0..height)
            .map(|j| {
                let (first, last) = (row(j)[0], row(j)[width - 1]);
                (
                    (first.0.min(last.0), first.0.max(last.0)),
                    (first.1.min(last.1), first.1.max(last.1)),
                )
            })
            .collect();

        for (t, owner) in self.triangles.iter().zip(1_u32..) {
            let rect = t.pixels.intersect(&reached);
            if rect.is_empty() {
                continue;
            }

            let rows = runs([
                spanning(&spans, |s| s.0, down_column.0, &rect.xs),
                spanning(&spans, |s| s.1, down_column.1, &rect.ys),
            ]);

            // Then the rows that meet the rectangle have their y in it, and
            // the same columns in it.
            let upright_columns =
                upright.then(|| spanning(row(0), |p| (p.0, p.0), along_row.0, &rect.xs));

            for j in rows {
                let row = row(j);
                let columns = upright_columns.clone().unwrap_or_else(|| {
                    runs([
                        spanning(row, |p| (p.0, p.0), along_row.0, &rect.xs),
                        spanning(row, |p| (p.1, p.1), along_row.1, &rect.ys),
                    ])
                });

                for i in columns {
                    let (x, y) = row[i];
                    if t.triangle.contains_point(Point::new(x as f64, y as f64)) {
                        owners[j * width + i] = owner;
                    }
                }
            }
        }

        Coverage { pixels, owners }
    }
}

/// The indices of the items whose span, from the least to the greatest value
/// `span` gives for the item, meets `values`, given that both ends of the
/// spans grow along the items if `growing`, and else shrink.
fn spanning<T>(
    items: &[T],
    span: impl Fn(&T) -> (u16, u16),
    growing: bool,
    values: &Range<u16>,
) -> Range<usize> {
    if growing {
        items.partition_point(|i| span(i).1 < values.start)
            ..items.partition_point(|i| span(i).0 < values.end)
    } else {
        items.partition_point(|i| span(i).0 >= values.end)
            ..items.partition_point(|i| span(i).1 >= values.start)
    }
}

/// The indices in all of `runs`.
fn runs<const N: usize>(runs: [Range<usize>; N]) -> Range<usize> {
    runs.into_iter()
        .reduce(|a, b| a.start.max(b.start)..a.end.min(b.end))
        .unwrap_or_default()
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
                let pixel = pixel(pos);
                shading.color(shading.triangle_at(pixel), pixel, bg_color, color_space)
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
    use super::{EncodedShadingPattern, EncodedShadingType, encode_mesh, texture_dimensions};
    use crate::color::{AlphaColor, ColorComponents, ColorSpace};
    use crate::shading::{Triangle, TriangleVertex};
    use crate::util::x_y_advances;
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

    const BACKGROUND: [f32; 4] = [0.1, 0.2, 0.3, 0.4];

    /// The pixels of the positions `sample_texture` samples, in its order.
    fn pixels(base_transform: Affine, bbox: Rect, scale: f32) -> Vec<(u16, u16)> {
        let (width, height) = texture_dimensions(bbox, scale);
        let inv_scale = 1.0 / scale as f64;
        let (x_advance, y_advance) = x_y_advances(&(Affine::scale(inv_scale) * base_transform));
        let mut start = base_transform
            * Affine::translate((0.5 * inv_scale, 0.5 * inv_scale))
            * Point::new(bbox.x0, bbox.y0);

        let mut pixels = vec![];
        for _ in 0..height {
            let mut point = start;
            for _ in 0..width {
                pixels.push((point.x as u16, point.y as u16));
                point += x_advance;
            }
            start += y_advance;
        }

        pixels
    }

    /// Sample `triangles`, placed in device space by `transform`, as a texture
    /// of `bbox` would be sampled by putting each covered pixel of each triangle
    /// into a map, the later triangle over the earlier, and looking up the pixel
    /// of each position, and check that `sample_texture` and `sample` give the
    /// same colors. Return the number of positions.
    fn check(
        triangles: &[Triangle],
        transform: Affine,
        base_transform: Affine,
        bbox: Rect,
        scale: f32,
    ) -> usize {
        let pixels = pixels(base_transform, bbox, scale);

        // Only the pixels the positions fall in are looked up, so the map
        // needs no others.
        let low = |c: fn(&(u16, u16)) -> u16| pixels.iter().map(c).min().unwrap() as u32;
        let high = |c: fn(&(u16, u16)) -> u16| pixels.iter().map(c).max().unwrap() as u32;
        let (xs, ys) = (
            low(|p| p.0)..high(|p| p.0) + 1,
            low(|p| p.1)..high(|p| p.1) + 1,
        );
        let mut map = FxHashMap::<(u16, u16), ColorComponents>::default();
        for t in triangles {
            let [p0, p1, p2] = [&t.p0, &t.p1, &t.p2].map(|v| {
                let mut v = (*v).clone();
                v.point = transform * v.point;
                v
            });
            let t = Triangle::new(p0, p1, p2);
            let bbox = t.bounding_box();
            let y0 = (bbox.y0.floor() as u16 as u32).max(ys.start);
            let y1 = (bbox.y1.ceil() as u16 as u32).min(ys.end);
            let x0 = (bbox.x0.floor() as u16 as u32).max(xs.start);
            let x1 = (bbox.x1.ceil() as u16 as u32).min(xs.end);
            for y in y0..y1 {
                for x in x0..x1 {
                    let point = Point::new(x as f64, y as f64);
                    if t.contains_point(point) {
                        map.insert((x as u16, y as u16), t.interpolate(point));
                    }
                }
            }
        }
        let color_space = ColorSpace::device_rgb();
        let expected = pixels.iter().map(|p| match map.get(p) {
            Some(color) => color_space.to_rgba(color, 1.0).components(),
            None => BACKGROUND,
        });

        let pattern = EncodedShadingPattern {
            base_transform,
            color_space: color_space.clone(),
            background_color: AlphaColor::new(BACKGROUND),
            shading_type: encode_mesh(triangles, transform, &None),
            opacity: 1.0,
            transfer_function: None,
        };
        let mut found = vec![];
        pattern.sample_texture(bbox, scale, |sample| found.push(sample));
        assert_eq!(found.len(), pixels.len());

        for (k, ((expected, found), &(x, y))) in expected.zip(&found).zip(&pixels).enumerate() {
            let at = || format!("{scale} {bbox:?} {base_transform:?}: {k} at ({x}, {y})");
            assert_eq!(*found, expected, "{}", at());
            // `sample` searches all triangles, so check it at some positions.
            if k % 7 == 0 {
                let searched = pattern.sample(Point::new(x as f64, y as f64));
                assert_eq!(searched, expected, "{}", at());
            }
        }

        pixels.len()
    }

    #[test]
    fn a_mesh_is_sampled_as_a_map_of_its_covered_pixels_would_hold_it() {
        let mut random = Random(0x2545_f491_4f6c_dd1d);
        // A tiling of a box, whose corners fall on pixels, with triangles over
        // it that cover others.
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
            for base_transform in [
                Affine::IDENTITY,
                Affine::rotate_about(0.5, (120.0, 120.0)),
                // Flipped, so that pixels shrink along a row.
                Affine::new([-1.0, 0.0, 0.0, 1.0, 240.0, 0.0]),
                Affine::new([0.9, -0.4, 0.7, 1.2, -30.0, 15.0]),
                // Sheared, so that only y changes along a row.
                Affine::new([1.0, 0.3, 0.0, 1.0, 0.0, -20.0]),
            ] {
                for bbox in [
                    Rect::new(0.0, 0.0, 240.0, 240.0),
                    // At scale 2, positions fall on the edges of pixels.
                    Rect::new(0.25, 0.25, 240.25, 240.25),
                    Rect::new(60.1, 90.7, 129.3, 139.9),
                    // Partly at negative positions, which fall in pixel 0.
                    Rect::new(-50.3, -20.0, 150.0, 130.0),
                    Rect::new(1000.0, 1000.0, 1010.0, 1010.0),
                ] {
                    for scale in [0.01, 0.3, 0.5, 1.0, 2.0] {
                        check(&tiles, transform, base_transform, bbox, scale);
                    }
                }
            }
        }

        // Slivers far apart.
        let slivers = [
            triangle([(1.0, 1.0), (3.0, 1.0), (1.0, 390.0)], 0.25),
            triangle([(380.0, 1.0), (383.0, 1.0), (380.0, 390.0)], 0.75),
        ];
        let bbox = Rect::new(0.0, 0.0, 400.0, 400.0);
        check(&slivers, Affine::IDENTITY, Affine::IDENTITY, bbox, 1.0);
        check(&[], Affine::IDENTITY, Affine::IDENTITY, bbox, 1.0);
    }

    #[test]
    fn a_mesh_is_sampled_only_at_the_positions_read() {
        // A diagonal sliver whose box grows with it, read in 400 by 300 pixels.
        let bbox = Rect::new(50.0, 20.0, 450.0, 320.0);
        for size in [100.0, 1000.0, 10000.0, 65534.0] {
            let sliver = [triangle(
                [(0.0, 0.0), (size, size), (size, size - 0.01)],
                0.5,
            )];
            check(&sliver, Affine::IDENTITY, Affine::IDENTITY, bbox, 1.0);
        }

        // Two small triangles in opposite corners of a large box, read at a
        // hundredth of its pixels.
        let corners = [
            triangle([(0.0, 0.0), (300.0, 0.0), (0.0, 300.0)], 0.25),
            triangle(
                [(59700.0, 59999.0), (59999.0, 59700.0), (59999.0, 59999.0)],
                0.75,
            ),
        ];
        let bbox = Rect::new(0.0, 0.0, 60000.0, 60000.0);
        assert_eq!(
            check(&corners, Affine::IDENTITY, Affine::IDENTITY, bbox, 0.01),
            600 * 600
        );

        let EncodedShadingType::Sampled(mesh) = encode_mesh(&corners, Affine::IDENTITY, &None)
        else {
            unreachable!()
        };
        let inv_scale = 1.0 / 0.01_f32 as f64;
        let coverage = mesh.coverage(
            Point::new(0.5 * inv_scale, 0.5 * inv_scale),
            kurbo::Vec2::new(inv_scale, 0.0),
            kurbo::Vec2::new(0.0, inv_scale),
            600,
            600,
        );
        assert_eq!(coverage.pixels.len(), 600 * 600);
        assert_eq!(coverage.owners.len(), 600 * 600);
        for owner in [1, 2] {
            assert!(coverage.owners.contains(&owner), "{owner}");
        }
    }
}
