//! Rasterization of function-based and mesh shadings.

use crate::color::ColorComponents;
use crate::encode::EncodedShadingPattern;
use crate::function::MAX_COMPONENTS;
use crate::gradient::{GradientLut, color_error, function_lut, lerp_color};
use crate::shading::{Patch, ShadingFunction, ShadingType, Triangle};
use kurbo::{Affine, Point, Rect};

/// Target edge length in pixels of the pieces a mesh is split into.
const MESH_SEGMENT: f64 = 8.0;
/// Initial cell size in pixels for function-based shadings.
const FUNCTION_CELL: usize = 16;
/// Largest color error a function-based cell may have before it is split further.
const FUNCTION_TOLERANCE: f32 = 0.01;
/// Size in pixels up to which function-based cells are sampled at every pixel.
const FUNCTION_PIXEL_CELL: usize = 4;
const MAX_DEPTH: u32 = 10;
/// Approximate maximum number of pieces a mesh is split into.
const MAX_PIECES: f64 = (1 << 20) as f64;

/// A grid of straight (non-premultiplied) RGBA pixels.
pub(crate) struct Canvas {
    width: usize,
    height: usize,
    pub(crate) pixels: Vec<[f32; 4]>,
}

impl Canvas {
    pub(crate) fn new(width: u32, height: u32, background: [f32; 4]) -> Self {
        let (width, height) = (width as usize, height as usize);

        Self {
            width,
            height,
            pixels: vec![background; width * height],
        }
    }

    fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f64, self.height as f64)
    }

    fn is_visible(&self, points: &[Point]) -> bool {
        bounds(points).overlaps(self.bounds())
    }

    /// Fill the pixel centers in the triangle; centers on an edge go to the triangle below or right.
    fn fill_triangle<const N: usize>(
        &mut self,
        points: [Point; 3],
        values: [[f32; N]; 3],
        shade: &impl Fn(&[f32; N]) -> [f32; 4],
    ) {
        let [p0, p1, p2] = points;
        let (e1, e2) = (p1 - p0, p2 - p0);
        let det = e1.x * e2.y - e1.y * e2.x;

        if det == 0.0 || !det.is_finite() {
            return;
        }

        let mut dx = [0.0; N];
        let mut dy = [0.0; N];

        for c in 0..N {
            let d1 = (values[1][c] - values[0][c]) as f64;
            let d2 = (values[2][c] - values[0][c]) as f64;
            dx[c] = ((d1 * e2.y - d2 * e1.y) / det) as f32;
            dy[c] = ((d2 * e1.x - d1 * e2.x) / det) as f32;
        }

        let mut sorted = points;
        sorted.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        let [top, middle, bottom] = sorted;

        let first_row = pixel_index(top.y, self.height);
        let end_row = pixel_index(bottom.y, self.height);

        for y in first_row..end_row {
            let center_y = y as f64 + 0.5;
            let long = edge_x(top, bottom, center_y);
            let short = if center_y < middle.y {
                edge_x(top, middle, center_y)
            } else {
                edge_x(middle, bottom, center_y)
            };

            let start = pixel_index(long.min(short), self.width);
            let end = pixel_index(long.max(short), self.width);

            if start >= end {
                continue;
            }

            let mut value = [0.0; N];
            let (offset_x, offset_y) = (start as f64 + 0.5 - p0.x, center_y - p0.y);

            for c in 0..N {
                value[c] =
                    values[0][c] + (dx[c] as f64 * offset_x + dy[c] as f64 * offset_y) as f32;
            }

            for pixel in &mut self.pixels[y * self.width + start..y * self.width + end] {
                *pixel = shade(&value);

                for c in 0..N {
                    value[c] += dx[c];
                }
            }
        }
    }
}

/// The first pixel whose center is at or past `coordinate`, clamped to `[0, len]`.
fn pixel_index(coordinate: f64, len: usize) -> usize {
    (coordinate - 0.5).ceil().clamp(0.0, len as f64) as usize
}

/// The x at which the edge `ab` crosses `y`, independent of the order of `a` and `b`.
fn edge_x(a: Point, b: Point, y: f64) -> f64 {
    let (a, b) = if (a.y, a.x) <= (b.y, b.x) {
        (a, b)
    } else {
        (b, a)
    };

    a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y)
}

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

/// Draw a function-based shading, with `transform` mapping its domain to the canvas.
pub(crate) fn draw_function(
    canvas: &mut Canvas,
    domain: Rect,
    function: &ShadingFunction,
    transform: Affine,
    pattern: &EncodedShadingPattern,
) {
    let area = transform
        .transform_rect_bbox(domain)
        .intersect(canvas.bounds());

    if area.width() <= 0.0 || area.height() <= 0.0 {
        return;
    }

    let sampler = FunctionSampler {
        function,
        inverse: transform.inverse(),
        domain,
        pattern,
    };
    let (x0, y0) = (area.x0.floor() as usize, area.y0.floor() as usize);
    let columns = (area.x1.ceil() as usize - x0).div_ceil(FUNCTION_CELL);
    let rows = (area.y1.ceil() as usize - y0).div_ceil(FUNCTION_CELL);
    let corner =
        |column: usize, row: usize| [x0 + column * FUNCTION_CELL, y0 + row * FUNCTION_CELL];
    let sample_row = |row: usize| {
        (0..=columns)
            .map(|column| sampler.sample(corner(column, row)))
            .collect::<Vec<_>>()
    };

    let mut previous = sample_row(0);

    for row in 1..=rows {
        let current = sample_row(row);

        for column in 0..columns {
            let corners = [
                previous[column],
                previous[column + 1],
                current[column],
                current[column + 1],
            ];

            draw_function_cell(
                canvas,
                &sampler,
                corner(column, row - 1),
                FUNCTION_CELL,
                corners,
            );
        }

        previous = current;
    }
}

struct FunctionSampler<'a> {
    function: &'a ShadingFunction,
    inverse: Affine,
    domain: Rect,
    pattern: &'a EncodedShadingPattern,
}

impl FunctionSampler<'_> {
    /// The color at the given pixel corner of the canvas.
    fn sample(&self, [x, y]: [usize; 2]) -> [f32; 4] {
        self.sample_at(Point::new(x as f64, y as f64))
    }

    fn sample_at(&self, point: Point) -> [f32; 4] {
        let point = self.inverse * point;
        let mut out = [0.0; MAX_COMPONENTS];
        let out = &mut out[..self.function.num_outputs()];

        match self.function.eval(&[point.x as f32, point.y as f32], out) {
            Some(()) => self.pattern.convert(out),
            None => [0.0; 4],
        }
    }

    fn contains(&self, point: Point) -> bool {
        self.domain.contains(self.inverse * point)
    }
}

/// Draw a cell from its corner colors (ordered by `y`, then `x`), refining it as needed.
fn draw_function_cell(
    canvas: &mut Canvas,
    sampler: &FunctionSampler<'_>,
    [x, y]: [usize; 2],
    size: usize,
    corners: [[f32; 4]; 4],
) {
    let corner_points = [[0, 0], [size, 0], [0, size], [size, size]]
        .map(|[dx, dy]| sampler.inverse * Point::new((x + dx) as f64, (y + dy) as f64));

    if x >= canvas.width || y >= canvas.height || !bounds(&corner_points).overlaps(sampler.domain) {
        return;
    }

    if size <= FUNCTION_PIXEL_CELL {
        for row in y..(y + size).min(canvas.height) {
            for column in x..(x + size).min(canvas.width) {
                let center = Point::new(column as f64 + 0.5, row as f64 + 0.5);

                if sampler.contains(center) {
                    canvas.pixels[row * canvas.width + column] = sampler.sample_at(center);
                }
            }
        }

        return;
    }

    let half = size / 2;
    let [c00, c10, c01, c11] = corners;
    let top = sampler.sample([x + half, y]);
    let left = sampler.sample([x, y + half]);
    let middle = sampler.sample([x + half, y + half]);
    let right = sampler.sample([x + size, y + half]);
    let bottom = sampler.sample([x + half, y + size]);

    let error = [
        color_error(top, lerp_color(c00, c10, 0.5)),
        color_error(left, lerp_color(c00, c01, 0.5)),
        color_error(right, lerp_color(c10, c11, 0.5)),
        color_error(bottom, lerp_color(c01, c11, 0.5)),
        color_error(
            middle,
            lerp_color(lerp_color(c00, c11, 0.5), lerp_color(c10, c01, 0.5), 0.5),
        ),
    ]
    .into_iter()
    .fold(0.0, f32::max);

    let quarters = [
        ([x, y], [c00, top, left, middle]),
        ([x + half, y], [top, c10, middle, right]),
        ([x, y + half], [left, middle, c01, bottom]),
        ([x + half, y + half], [middle, right, bottom, c11]),
    ];

    for (origin, corners) in quarters {
        if error > FUNCTION_TOLERANCE {
            draw_function_cell(canvas, sampler, origin, half, corners);
        } else {
            fill_function_cell(canvas, sampler, origin, half, corners);
        }
    }
}

/// Fill the pixels of a cell that lie in the domain by interpolating its corner colors.
fn fill_function_cell(
    canvas: &mut Canvas,
    sampler: &FunctionSampler<'_>,
    [x, y]: [usize; 2],
    size: usize,
    [c00, c10, c01, c11]: [[f32; 4]; 4],
) {
    let inside = [[0, 0], [size, 0], [0, size], [size, size]]
        .into_iter()
        .all(|[dx, dy]| sampler.contains(Point::new((x + dx) as f64, (y + dy) as f64)));
    let position = |offset: usize| (offset as f32 + 0.5) / size as f32;

    for row in y..(y + size).min(canvas.height) {
        let v = position(row - y);
        let (left, right) = (lerp_color(c00, c01, v), lerp_color(c10, c11, v));

        for column in x..(x + size).min(canvas.width) {
            let center = Point::new(column as f64 + 0.5, row as f64 + 0.5);

            if inside || sampler.contains(center) {
                canvas.pixels[row * canvas.width + column] =
                    lerp_color(left, right, position(column - x));
            }
        }
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

fn bounds(points: &[Point]) -> Rect {
    points
        .iter()
        .fold(Rect::from_points(points[0], points[0]), |bounds, point| {
            bounds.union_pt(*point)
        })
}

/// How `N` per-corner values are interpolated across a mesh and turned into colors.
trait MeshColors<const N: usize> {
    fn values(&self, components: &[f32]) -> [f32; N];
    fn shade(&self, values: &[f32; N]) -> [f32; 4];
}

/// Colors given by the input of a 1-in function, looked up in a table.
struct Parameter {
    lut: GradientLut,
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
