use crate::canvas::{Canvas, bounds};
use crate::color::ColorComponents;
use crate::encode::EncodedShadingPattern;
use crate::function::MAX_COMPONENTS;
use crate::gradient::{ColorLut, function_lut};
use crate::shading::{Patch, ShadingFunction, ShadingType, Triangle};
use kurbo::{Affine, Point, Rect};

const MESH_SEGMENT: f64 = 8.0;
const MAX_DEPTH: u32 = 10;
const MAX_PIECES: f64 = (1 << 20) as f64;

pub(crate) fn draw_mesh(
    canvas: &mut Canvas,
    shading_type: &ShadingType,
    transform: Affine,
    pattern: &EncodedShadingPattern,
) {
    match shading_type {
        ShadingType::TriangleMesh {
            triangles,
            function,
        } => {
            let triangles = triangles
                .iter()
                .map(|triangle| CanvasTriangle::new(triangle, transform))
                .collect::<Vec<_>>();

            if let Some(function) = function {
                let corner_colors = triangles.iter().flat_map(|t| t.colors);
                let colors = Parameter::new(function, corner_colors, pattern);
                draw_triangles(canvas, &triangles, 0, &colors);
            } else {
                let colors = Direct { pattern };
                let depth = if colors.is_linear() {
                    0
                } else {
                    let pieces = triangles.iter().map(|t| (t.extent(), t.bounds()));
                    mesh_depth(canvas.bounds(), pieces)
                };

                draw_triangles(canvas, &triangles, depth, &colors);
            }
        }
        ShadingType::PatchMesh { patches, function } => {
            let patches = patches
                .iter()
                .map(|patch| CanvasPatch::new(patch, transform))
                .collect::<Vec<_>>();
            let pieces = patches.iter().map(|p| (p.extent(), p.bounds()));
            let depth = mesh_depth(canvas.bounds(), pieces);

            if let Some(function) = function {
                let corner_colors = patches.iter().flat_map(|p| p.colors.as_flattened());
                let colors = Parameter::new(function, corner_colors, pattern);
                draw_patches(canvas, &patches, depth, &colors, |_, c| colors.values(c));
            } else {
                let colors = Direct { pattern };
                if pattern.color_space.is_lab() && depth > 1 {
                    let slots = ((1 << depth) + 1)
                        .min(canvas.width.saturating_add(canvas.height).saturating_add(1));
                    let mut columns = vec![None; slots];
                    draw_patches(canvas, &patches, depth, &colors, |i, c| {
                        let key = pattern.color_space.encode_values(c);
                        let slot = &mut columns[i % slots];
                        if let Some((old, rgba)) = slot
                            && *old == key
                        {
                            return *rgba;
                        }
                        let Some(color) = pattern.color_space.encoded_to_alpha_color(&key, 1.)
                        else {
                            return pattern.finish(crate::color::AlphaColor::BLACK.components());
                        };
                        let rgba = pattern.finish(color.components());
                        *slot = Some((key, rgba));
                        rgba
                    });
                } else {
                    draw_patches(canvas, &patches, depth, &colors, |_, c| colors.values(c));
                }
            }
        }
        _ => {}
    }
}

fn mesh_depth(canvas: Rect, pieces: impl Iterator<Item = (f64, Rect)>) -> u32 {
    let mut max_extent: f64 = 0.0;
    let mut visible_area = 0.0;

    for (extent, bounds) in pieces {
        if !bounds.overlaps(canvas) {
            continue;
        }

        let area = bounds.area();
        max_extent = max_extent.max(extent);
        visible_area += if area > 0.0 {
            bounds.intersect(canvas).area() / area
        } else {
            1.0
        };
    }

    let mut depth = (max_extent / MESH_SEGMENT)
        .log2()
        .ceil()
        .clamp(0.0, MAX_DEPTH as f64) as u32;

    while depth > 0 && visible_area * 4.0_f64.powi(depth as i32) > MAX_PIECES {
        depth -= 1;
    }

    depth
}

trait MeshColors<const N: usize> {
    fn values(&self, components: &[f32]) -> [f32; N];
    fn shade(&self, values: &[f32; N]) -> [f32; 4];
}

struct Parameter {
    lut: ColorLut,
    t0: f32,
    scale: f32,
}

impl Parameter {
    fn new<'a>(
        function: &ShadingFunction,
        colors: impl Iterator<Item = &'a ColorComponents>,
        pattern: &EncodedShadingPattern,
    ) -> Self {
        let (t0, t1) = colors
            .filter_map(|c| c.first().copied())
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), t| {
                (lo.min(t), hi.max(t))
            });
        let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (0.0, 0.0) };

        Self {
            lut: function_lut(function, [t0, t1], pattern),
            t0,
            scale: if t1 > t0 { 1.0 / (t1 - t0) } else { 0.0 },
        }
    }
}

impl MeshColors<1> for Parameter {
    fn values(&self, components: &[f32]) -> [f32; 1] {
        [(components.first().copied().unwrap_or(0.0) - self.t0) * self.scale]
    }

    fn shade(&self, values: &[f32; 1]) -> [f32; 4] {
        self.lut.get(values[0])
    }
}

struct Direct<'a> {
    pattern: &'a EncodedShadingPattern,
}

impl Direct<'_> {
    fn is_linear(&self) -> bool {
        self.pattern.color_space.is_device_gray_or_rgb() && self.pattern.transfer_function.is_none()
    }
}

impl MeshColors<4> for Direct<'_> {
    fn values(&self, components: &[f32]) -> [f32; 4] {
        self.pattern.convert(components)
    }

    fn shade(&self, values: &[f32; 4]) -> [f32; 4] {
        *values
    }
}

fn mix<'a, const K: usize>(
    colors: [&ColorComponents; K],
    weights: [f32; K],
    out: &'a mut [f32; MAX_COMPONENTS],
) -> &'a [f32] {
    let len = colors
        .iter()
        .map(|c| c.len())
        .min()
        .unwrap_or(0)
        .min(MAX_COMPONENTS);

    for (i, out) in out[..len].iter_mut().enumerate() {
        *out = colors
            .iter()
            .zip(weights)
            .map(|(color, weight)| color[i] * weight)
            .sum();
    }

    &out[..len]
}

struct CanvasTriangle<'a> {
    points: [Point; 3],
    colors: [&'a ColorComponents; 3],
}

impl<'a> CanvasTriangle<'a> {
    fn new(triangle: &'a Triangle, transform: Affine) -> Self {
        Self {
            points: triangle.vertices.each_ref().map(|v| transform * v.point),
            colors: triangle.vertices.each_ref().map(|v| &v.colors),
        }
    }

    fn extent(&self) -> f64 {
        let [a, b, c] = self.points;
        (a - b).hypot().max((b - c).hypot()).max((c - a).hypot())
    }

    fn bounds(&self) -> Rect {
        bounds(&self.points)
    }
}

fn draw_triangles<const N: usize>(
    canvas: &mut Canvas,
    triangles: &[CanvasTriangle<'_>],
    depth: u32,
    colors: &impl MeshColors<N>,
) {
    const CORNERS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for triangle in triangles {
        draw_triangle_piece(canvas, triangle, triangle.points, CORNERS, depth, colors);
    }
}

fn draw_triangle_piece<const N: usize>(
    canvas: &mut Canvas,
    triangle: &CanvasTriangle<'_>,
    points: [Point; 3],
    weights: [[f32; 3]; 3],
    depth: u32,
    colors: &impl MeshColors<N>,
) {
    if !canvas.is_visible(&points) {
        return;
    }

    if depth == 0 {
        let mut buffer = [0.0; MAX_COMPONENTS];
        let values = weights.map(|w| colors.values(mix(triangle.colors, w, &mut buffer)));
        canvas.fill_triangle(points, values, &|v| colors.shade(v));

        return;
    }

    let [p0, p1, p2] = points;
    let [w0, w1, w2] = weights;
    let (m01, m12, m20) = (p0.midpoint(p1), p1.midpoint(p2), p2.midpoint(p0));
    let mid = |a: [f32; 3], b: [f32; 3]| [0, 1, 2].map(|i| 0.5 * (a[i] + b[i]));
    let (n01, n12, n20) = (mid(w0, w1), mid(w1, w2), mid(w2, w0));

    for (points, weights) in [
        ([p0, m01, m20], [w0, n01, n20]),
        ([m01, p1, m12], [n01, w1, n12]),
        ([m20, m12, p2], [n20, n12, w2]),
        ([m01, m12, m20], [n01, n12, n20]),
    ] {
        draw_triangle_piece(canvas, triangle, points, weights, depth - 1, colors);
    }
}

struct CanvasPatch<'a> {
    points: [[Point; 4]; 4],
    colors: &'a [[ColorComponents; 2]; 2],
}

impl<'a> CanvasPatch<'a> {
    fn new(patch: &'a Patch, transform: Affine) -> Self {
        Self {
            points: patch
                .control_points
                .map(|column| column.map(|p| transform * p)),
            colors: &patch.colors,
        }
    }

    fn extent(&self) -> f64 {
        let length = |points: [Point; 4]| points.windows(2).map(|w| (w[1] - w[0]).hypot()).sum();
        let columns = self.points.map(length);
        let rows = [0, 1, 2, 3].map(|j| length(self.points.map(|column| column[j])));

        columns.into_iter().chain(rows).fold(0.0, f64::max)
    }

    fn bounds(&self) -> Rect {
        bounds(self.points.as_flattened())
    }

    fn components<'b>(&self, u: f32, v: f32, out: &'b mut [f32; MAX_COMPONENTS]) -> &'b [f32] {
        let [[c00, c01], [c10, c11]] = self.colors;
        let weights = [(1.0 - u) * (1.0 - v), (1.0 - u) * v, u * (1.0 - v), u * v];

        mix([c00, c01, c10, c11], weights, out)
    }
}

fn draw_patches<const N: usize>(
    canvas: &mut Canvas,
    patches: &[CanvasPatch<'_>],
    depth: u32,
    colors: &impl MeshColors<N>,
    mut values: impl FnMut(usize, &[f32]) -> [f32; N],
) {
    for patch in patches {
        draw_patch_rows(
            canvas,
            patch,
            patch.points,
            [0., 1.],
            depth,
            depth,
            colors,
            &mut values,
        );
    }
}

fn draw_patch_rows<const N: usize>(
    canvas: &mut Canvas,
    patch: &CanvasPatch<'_>,
    points: [[Point; 4]; 4],
    [v0, v1]: [f32; 2],
    remaining: u32,
    depth: u32,
    colors: &impl MeshColors<N>,
    values: &mut impl FnMut(usize, &[f32]) -> [f32; N],
) {
    if !canvas.is_visible(points.as_flattened()) {
        return;
    }
    if remaining > 0 {
        let (lo, hi) = split_rows(points);
        let mid = (v0 + v1) * 0.5;
        for (points, range) in [(lo, [v0, mid]), (hi, [mid, v1])] {
            draw_patch_rows(
                canvas,
                patch,
                points,
                range,
                remaining - 1,
                depth,
                colors,
                values,
            );
        }
        return;
    }
    let segments = 1_usize << depth;
    let mut previous = None;
    let mut buffer = [0.; MAX_COMPONENTS];
    let mut column = |i: usize| -> [[f32; N]; 2] {
        let u = i as f32 / segments as f32;
        [v0, v1].map(|v| values(i, patch.components(u, v, &mut buffer)))
    };
    draw_patch_strip(
        canvas,
        points.map(|c| c[0]),
        points.map(|c| c[3]),
        0,
        segments,
        &mut |canvas, i, p, q| {
            let [c00, c01] = match previous {
                Some((last, values)) if last == i => values,
                _ => column(i),
            };
            let right = column(i + 1);
            let [c10, c11] = right;
            previous = Some((i + 1, right));
            let shade = |v: &[f32; N]| colors.shade(v);
            canvas.fill_triangle([p[0], p[1], q[0]], [c00, c10, c01], &shade);
            canvas.fill_triangle([p[1], q[1], q[0]], [c10, c11, c01], &shade);
        },
    );
}

fn draw_patch_strip(
    canvas: &mut Canvas,
    top: [Point; 4],
    bottom: [Point; 4],
    start: usize,
    width: usize,
    leaf: &mut impl FnMut(&mut Canvas, usize, [Point; 2], [Point; 2]),
) {
    let area = if width == 1 {
        bounds(&[top[0], top[3], bottom[0], bottom[3]])
    } else {
        bounds(&top).union(bounds(&bottom))
    };
    if !area.overlaps(canvas.bounds()) || (area.y0 - 0.5).ceil() >= (area.y1 - 0.5).ceil() {
        return;
    }
    if width == 1 {
        return leaf(canvas, start, [top[0], top[3]], [bottom[0], bottom[3]]);
    }
    let (a, b) = split_cubic(top);
    let (c, d) = split_cubic(bottom);
    let half = width / 2;
    draw_patch_strip(canvas, a, c, start, half, leaf);
    draw_patch_strip(canvas, b, d, start + half, half, leaf);
}

fn split_rows(points: [[Point; 4]; 4]) -> ([[Point; 4]; 4], [[Point; 4]; 4]) {
    let split = points.map(split_cubic);

    (split.map(|(low, _)| low), split.map(|(_, high)| high))
}

fn split_cubic([p0, p1, p2, p3]: [Point; 4]) -> ([Point; 4], [Point; 4]) {
    let (p01, p12, p23) = (p0.midpoint(p1), p1.midpoint(p2), p2.midpoint(p3));
    let (p012, p123) = (p01.midpoint(p12), p12.midpoint(p23));
    let p0123 = p012.midpoint(p123);

    ([p0, p01, p012, p0123], [p0123, p123, p23, p3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaColor, ColorSpace};
    use crate::encode::{EncodedMeshShading, EncodedShadingType};

    #[test]
    fn device_gray_half_byte_patch() {
        let patch = Patch {
            control_points: std::array::from_fn(|i| {
                std::array::from_fn(|j| Point::new(0.5 + 8.0 * i as f64, 0.5 + 8.0 * j as f64))
            }),
            colors: std::array::from_fn(|_| {
                std::array::from_fn(|_| smallvec::smallvec![59.0 / 510.0])
            }),
        };
        let pattern = EncodedShadingPattern {
            base_transform: Affine::IDENTITY,
            color_space: ColorSpace::device_gray(),
            background_color: AlphaColor::BLACK,
            shading_type: EncodedShadingType::Mesh(EncodedMeshShading {
                shading_type: ShadingType::PatchMesh {
                    patches: vec![patch],
                    function: None,
                }
                .into(),
            }),
            opacity: 1.,
            transfer_function: None,
        };
        let texture = pattern.render_texture(Rect::new(0., 0., 24., 24.), 1.);
        let mut expected = [[30, 30, 30, 255]; 24 * 24];
        for (y, columns) in [
            (4, 18..21),
            (5, 17..21),
            (6, 16..21),
            (7, 16..20),
            (8, 16..19),
        ] {
            expected[y * 24 + columns.start..y * 24 + columns.end].fill([29, 29, 29, 255]);
        }
        assert_eq!(texture.data, expected.as_flattened());
    }

    #[test]
    fn shared_patch_edge_is_not_culled_at_pixel_center() {
        let patch = |p00: Point, p10: Point, p01: Point, p11: Point| Patch {
            control_points: [
                [p00, p00, p10, p10],
                [p00, p00, p10, p10],
                [p01, p01, p11, p11],
                [p01, p01, p11, p11],
            ],
            colors: std::array::from_fn(|_| std::array::from_fn(|_| smallvec::smallvec![1.0])),
        };
        let left = patch(
            Point::new(-1.0 / 32.0, 0.0),
            Point::new(0.0, 0.0),
            Point::new(-1.0 / 32.0, 1.0),
            Point::new(0.0, 1.0),
        );
        let right = patch(
            Point::new(0.0, 0.0),
            Point::new(7.0 / 16.0, 2.0),
            Point::new(0.0, 1.0),
            Point::new(7.0 / 16.0, 3.0),
        );
        let shading_type = std::sync::Arc::new(ShadingType::PatchMesh {
            patches: vec![left, right],
            function: None,
        });
        let pattern = EncodedShadingPattern {
            base_transform: Affine::IDENTITY,
            color_space: ColorSpace::device_gray(),
            background_color: AlphaColor::TRANSPARENT,
            shading_type: EncodedShadingType::Mesh(EncodedMeshShading {
                shading_type: shading_type.clone(),
            }),
            opacity: 1.,
            transfer_function: None,
        };
        let transform = Affine::new([
            1.0,
            0.0,
            0.775_396_347_045_898_4,
            1.394_090_960_019_853_7,
            -0.275_396_347_045_898_44,
            -0.894_090_960_019_853_6,
        ]);
        let mut canvas = Canvas::new(1, 1, [0.0; 4]);
        draw_mesh(&mut canvas, &shading_type, transform, &pattern);
        assert_eq!(canvas.pixels, [[1.0, 1.0, 1.0, 1.0]]);
    }
}
