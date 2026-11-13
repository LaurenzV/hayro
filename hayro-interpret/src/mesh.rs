//! Rasterization of mesh shadings.

use crate::canvas::{Canvas, bounds};
use crate::color::ColorComponents;
use crate::encode::EncodedShadingPattern;
use crate::function::MAX_COMPONENTS;
use crate::gradient::{ColorLut, function_lut};
use crate::shading::{Patch, ShadingFunction, ShadingType, Triangle};
use kurbo::{Affine, Point, Rect};

/// Target edge length in pixels of the pieces a mesh is split into.
const MESH_SEGMENT: f64 = 8.0;
const MAX_DEPTH: u32 = 10;
/// Approximate maximum number of pieces a mesh is split into.
const MAX_PIECES: f64 = (1 << 20) as f64;

/// Draw a mesh shading, with `transform` mapping shading space to the canvas.
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
                draw_patches(canvas, &patches, depth, &colors);
            } else {
                draw_patches(canvas, &patches, depth, &Direct { pattern });
            }
        }
        _ => {}
    }
}

/// The depth to which all pieces are split; one shared depth keeps shared edges crack-free.
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

/// How `N` per-corner values are interpolated across a mesh and turned into colors.
trait MeshColors<const N: usize> {
    fn values(&self, components: &[f32]) -> [f32; N];
    fn shade(&self, values: &[f32; N]) -> [f32; 4];
}

/// Colors given by the input of a 1-in function, looked up in a table.
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

/// Colors converted from components at the corners and interpolated as RGBA.
struct Direct<'a> {
    pattern: &'a EncodedShadingPattern,
}

impl Direct<'_> {
    /// Whether interpolating converted colors equals converting interpolated components.
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

/// Mix color components with the given weights.
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

/// A triangle of a mesh, in canvas space.
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

/// Draw the piece of `triangle` with the given corners and barycentric `weights`.
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

/// A patch of a mesh, in canvas space.
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

    /// The length of the longest row or column of the control net.
    fn extent(&self) -> f64 {
        let length = |points: [Point; 4]| points.windows(2).map(|w| (w[1] - w[0]).hypot()).sum();
        let columns = self.points.map(length);
        let rows = [0, 1, 2, 3].map(|j| length(self.points.map(|column| column[j])));

        columns.into_iter().chain(rows).fold(0.0, f64::max)
    }

    fn bounds(&self) -> Rect {
        bounds(self.points.as_flattened())
    }

    /// The color components at the given position within the patch.
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
) {
    for patch in patches {
        let ranges = [[0.0, 1.0]; 2];
        draw_patch_piece(canvas, patch, patch.points, ranges, [depth; 2], colors);
    }
}

/// Draw a patch piece, splitting along `v` before `u` so that larger `v`, then `u`, paints last.
fn draw_patch_piece<const N: usize>(
    canvas: &mut Canvas,
    patch: &CanvasPatch<'_>,
    points: [[Point; 4]; 4],
    [u_range, v_range]: [[f32; 2]; 2],
    [u_depth, v_depth]: [u32; 2],
    colors: &impl MeshColors<N>,
) {
    if !canvas.is_visible(points.as_flattened()) {
        return;
    }

    let halves = |[a, b]: [f32; 2]| [[a, 0.5 * (a + b)], [0.5 * (a + b), b]];

    if v_depth > 0 {
        let (low, high) = split_rows(points);

        for (points, v_range) in [low, high].into_iter().zip(halves(v_range)) {
            let ranges = [u_range, v_range];
            draw_patch_piece(
                canvas,
                patch,
                points,
                ranges,
                [u_depth, v_depth - 1],
                colors,
            );
        }
    } else if u_depth > 0 {
        let (low, high) = split_columns(points);

        for (points, u_range) in [low, high].into_iter().zip(halves(u_range)) {
            let ranges = [u_range, v_range];
            draw_patch_piece(
                canvas,
                patch,
                points,
                ranges,
                [u_depth - 1, v_depth],
                colors,
            );
        }
    } else {
        let ([u0, u1], [v0, v1]) = (u_range, v_range);
        let mut buffer = [0.0; MAX_COMPONENTS];
        let mut values = |u, v| colors.values(patch.components(u, v, &mut buffer));
        let (c00, c10, c01, c11) = (
            values(u0, v0),
            values(u1, v0),
            values(u0, v1),
            values(u1, v1),
        );
        let (p00, p10, p01, p11) = (points[0][0], points[3][0], points[0][3], points[3][3]);
        let shade = |v: &[f32; N]| colors.shade(v);

        canvas.fill_triangle([p00, p10, p01], [c00, c10, c01], &shade);
        canvas.fill_triangle([p10, p11, p01], [c10, c11, c01], &shade);
    }
}

/// Split a control net at `v = 0.5`.
fn split_rows(points: [[Point; 4]; 4]) -> ([[Point; 4]; 4], [[Point; 4]; 4]) {
    let split = points.map(split_cubic);

    (split.map(|(low, _)| low), split.map(|(_, high)| high))
}

/// Split a control net at `u = 0.5`.
fn split_columns(points: [[Point; 4]; 4]) -> ([[Point; 4]; 4], [[Point; 4]; 4]) {
    let mut low = points;
    let mut high = points;

    for j in 0..4 {
        let (l, h) = split_cubic(points.map(|column| column[j]));

        for i in 0..4 {
            low[i][j] = l[i];
            high[i][j] = h[i];
        }
    }

    (low, high)
}

/// Split a cubic Bézier curve at `t = 0.5`.
fn split_cubic([p0, p1, p2, p3]: [Point; 4]) -> ([Point; 4], [Point; 4]) {
    let (p01, p12, p23) = (p0.midpoint(p1), p1.midpoint(p2), p2.midpoint(p3));
    let (p012, p123) = (p01.midpoint(p12), p12.midpoint(p23));
    let p0123 = p012.midpoint(p123);

    ([p0, p01, p012, p0123], [p0123, p123, p23, p3])
}
