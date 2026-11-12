//! Rasterization of function-based and mesh shadings.
//!
//! All of these shadings are reduced to triangles whose corners carry either a final color or a
//! normalized function input, which are then filled into a [`Canvas`] with linear interpolation
//! across each triangle.
//!
//! Triangles and patches are refined by repeated halving, using the same depth for the whole
//! mesh. Since midpoints are computed symmetrically, two neighbors split their shared edge into
//! exactly the same points, which keeps the mesh free of cracks.

use crate::color::ColorComponents;
use crate::encode::EncodedShadingPattern;
use crate::function::MAX_COMPONENTS;
use crate::gradient::{GradientLut, color_error, finish_color, function_lut, lerp_color};
use crate::shading::{Patch, ShadingFunction, ShadingType, Triangle};
use kurbo::{Affine, Point, Rect};

/// The target edge length in pixels of the pieces meshes are split into.
const MESH_SEGMENT: f64 = 8.0;
/// The size in pixels of the cells function-based shadings are sampled in before refinement.
const FUNCTION_CELL: f64 = 16.0;
/// The largest error in a color component that the interpolation of a function-based shading
/// may introduce before cells are refined further.
const FUNCTION_TOLERANCE: f32 = 0.01;
/// The maximum number of times the pieces of a mesh are halved.
const MAX_DEPTH: u32 = 10;
/// The maximum number of pieces a mesh is split into, approximately.
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

    /// Fill the pixels whose centers lie inside the triangle, interpolating `values` linearly
    /// between the corners and turning them into colors with `shade`.
    ///
    /// A pixel center on an edge belongs to the triangle only if the triangle lies below or to the
    /// right of it, so triangles sharing an edge never both fill a pixel or both miss it.
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

/// The index of the first pixel whose center is not before `coordinate`, clamped to `[0, len]`.
fn pixel_index(coordinate: f64, len: usize) -> usize {
    (coordinate - 0.5).ceil().clamp(0.0, len as f64) as usize
}

/// The x coordinate at which the edge between `a` and `b` crosses the line at `y`.
///
/// The endpoints are put into a fixed order first, so that both triangles sharing an edge get
/// exactly the same result.
fn edge_x(a: Point, b: Point, y: f64) -> f64 {
    let (a, b) = if (a.y, a.x) <= (b.y, b.x) {
        (a, b)
    } else {
        (b, a)
    };

    a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y)
}

/// Draw a triangle-mesh or patch-mesh shading, given the transform from shading space to the
/// canvas.
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
            let pieces = patches
                .iter()
                .map(|p| (p.extent(), bounds(p.points.as_flattened())));
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

/// Draw a function-based shading over `domain`, given the transform from the domain to the
/// canvas.
pub(crate) fn draw_function(
    canvas: &mut Canvas,
    domain: Rect,
    function: &ShadingFunction,
    transform: Affine,
    pattern: &EncodedShadingPattern,
) {
    let visible = transform
        .inverse()
        .transform_rect_bbox(canvas.bounds())
        .intersect(domain);

    if visible.width() <= 0.0 || visible.height() <= 0.0 {
        return;
    }

    let sampler = FunctionSampler {
        function,
        transform,
        pattern,
    };
    let length = |a: Point, b: Point| (transform * a - transform * b).hypot();
    let origin = visible.origin();
    let columns = (length(origin, Point::new(visible.x1, visible.y0)) / FUNCTION_CELL).ceil();
    let rows = (length(origin, Point::new(visible.x0, visible.y1)) / FUNCTION_CELL).ceil();
    let (columns, rows) = (columns.max(1.0) as usize, rows.max(1.0) as usize);
    let x = |column: usize| visible.x0 + visible.width() * column as f64 / columns as f64;
    let y = |row: usize| visible.y0 + visible.height() * row as f64 / rows as f64;
    let sample_row = |row: usize| {
        (0..=columns)
            .map(|column| sampler.sample(Point::new(x(column), y(row))))
            .collect::<Vec<_>>()
    };

    let mut previous = sample_row(0);

    for row in 1..=rows {
        let current = sample_row(row);

        for column in 0..columns {
            let cell = Rect::new(x(column), y(row - 1), x(column + 1), y(row));
            let corners = [
                previous[column],
                previous[column + 1],
                current[column],
                current[column + 1],
            ];

            draw_function_cell(canvas, &sampler, cell, corners);
        }

        previous = current;
    }
}

struct FunctionSampler<'a> {
    function: &'a ShadingFunction,
    transform: Affine,
    pattern: &'a EncodedShadingPattern,
}

impl FunctionSampler<'_> {
    fn sample(&self, point: Point) -> [f32; 4] {
        let mut out = [0.0; MAX_COMPONENTS];
        let out = &mut out[..self.function.num_outputs().min(MAX_COMPONENTS)];

        if self
            .function
            .eval(&[point.x as f32, point.y as f32], out)
            .is_some()
        {
            let color = self.pattern.color_space.to_rgba(out, 1.0);
            finish_color(self.pattern, color.components())
        } else {
            [0.0; 4]
        }
    }
}

/// Draw a cell of a function-based shading, given the colors at its corners, ordered by `y`
/// and then by `x`.
///
/// The cell is split as long as its colors deviate from the interpolation of its corners by
/// more than [`FUNCTION_TOLERANCE`], until it covers about a pixel.
fn draw_function_cell(
    canvas: &mut Canvas,
    sampler: &FunctionSampler<'_>,
    cell: Rect,
    corners: [[f32; 4]; 4],
) {
    let transform = sampler.transform;
    let points = [
        Point::new(cell.x0, cell.y0),
        Point::new(cell.x1, cell.y0),
        Point::new(cell.x0, cell.y1),
        Point::new(cell.x1, cell.y1),
    ]
    .map(|p| transform * p);

    if !overlaps(bounds(&points), canvas.bounds()) {
        return;
    }

    let [c00, c10, c01, c11] = corners;
    let center = cell.center();
    let top = sampler.sample(Point::new(center.x, cell.y0));
    let left = sampler.sample(Point::new(cell.x0, center.y));
    let middle = sampler.sample(center);
    let right = sampler.sample(Point::new(cell.x1, center.y));
    let bottom = sampler.sample(Point::new(center.x, cell.y1));

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

    let [p00, p10, p01, _] = points;
    let size = (p10 - p00).hypot().max((p01 - p00).hypot());
    let quarters = [
        (
            Rect::new(cell.x0, cell.y0, center.x, center.y),
            [c00, top, left, middle],
        ),
        (
            Rect::new(center.x, cell.y0, cell.x1, center.y),
            [top, c10, middle, right],
        ),
        (
            Rect::new(cell.x0, center.y, center.x, cell.y1),
            [left, middle, c01, bottom],
        ),
        (
            Rect::new(center.x, center.y, cell.x1, cell.y1),
            [middle, right, bottom, c11],
        ),
    ];

    if error > FUNCTION_TOLERANCE && size > 1.0 {
        for (quarter, corners) in quarters {
            draw_function_cell(canvas, sampler, quarter, corners);
        }
    } else {
        for (quarter, [c00, c10, c01, c11]) in quarters {
            let [p00, p10, p01, p11] = [
                Point::new(quarter.x0, quarter.y0),
                Point::new(quarter.x1, quarter.y0),
                Point::new(quarter.x0, quarter.y1),
                Point::new(quarter.x1, quarter.y1),
            ]
            .map(|p| transform * p);

            canvas.fill_triangle([p00, p10, p01], [c00, c10, c01], &|c| *c);
            canvas.fill_triangle([p10, p11, p01], [c10, c11, c01], &|c| *c);
        }
    }
}

/// The depth to which all pieces of a mesh are split, given the extent and bounds of each piece.
///
/// The depth is chosen such that the largest visible piece is split into segments of about
/// [`MESH_SEGMENT`] pixels, as long as the estimated number of visible pieces stays within
/// [`MAX_PIECES`].
fn mesh_depth(canvas: Rect, pieces: impl Iterator<Item = (f64, Rect)>) -> u32 {
    let mut max_extent: f64 = 0.0;
    let mut visible_area = 0.0;

    for (extent, bounds) in pieces {
        if !overlaps(bounds, canvas) {
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

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && a.x1 >= b.x0 && a.y0 <= b.y1 && a.y1 >= b.y0
}

fn bounds(points: &[Point]) -> Rect {
    points
        .iter()
        .fold(Rect::from_points(points[0], points[0]), |bounds, point| {
            bounds.union_pt(*point)
        })
}

/// The way the colors of a mesh are interpolated.
///
/// Corners carry `N` values, which are interpolated linearly and turned into colors per pixel.
trait MeshColors<const N: usize> {
    /// The values for a corner with the given color components.
    fn values(&self, components: &[f32]) -> [f32; N];
    /// The color for interpolated values.
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

/// Colors given directly as components in the color space of the shading.
///
/// The components are converted at the corners, and the resulting colors are interpolated.
struct Direct<'a> {
    pattern: &'a EncodedShadingPattern,
}

impl Direct<'_> {
    /// Whether interpolating converted colors gives the same result as converting interpolated
    /// components.
    fn is_linear(&self) -> bool {
        self.pattern.color_space.is_device_gray_or_rgb() && self.pattern.transfer_function.is_none()
    }
}

impl MeshColors<4> for Direct<'_> {
    fn values(&self, components: &[f32]) -> [f32; 4] {
        let color = self.pattern.color_space.to_rgba(components, 1.0);
        finish_color(self.pattern, color.components())
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

/// Draw the piece of `triangle` with the given corners, whose barycentric coordinates within the
/// triangle are `weights`, after splitting it `depth` more times.
fn draw_triangle_piece<const N: usize>(
    canvas: &mut Canvas,
    triangle: &CanvasTriangle<'_>,
    points: [Point; 3],
    weights: [[f32; 3]; 3],
    depth: u32,
    colors: &impl MeshColors<N>,
) {
    if !overlaps(bounds(&points), canvas.bounds()) {
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
        draw_patch_rows(canvas, patch, patch.points, [0.0, 1.0], [depth; 2], colors);
    }
}

/// Split the part `v_range` of a patch, whose control net is `points`, into rows, which are then
/// split into pieces, and draw them in order of increasing `v`. `depth` holds the number of
/// remaining splits along `v` and along `u`.
///
/// Drawing rows of larger `v`, and within a row pieces of larger `u`, later makes them win where
/// a patch folds over itself.
fn draw_patch_rows<const N: usize>(
    canvas: &mut Canvas,
    patch: &CanvasPatch<'_>,
    points: [[Point; 4]; 4],
    v_range: [f32; 2],
    [v_depth, u_depth]: [u32; 2],
    colors: &impl MeshColors<N>,
) {
    if !overlaps(bounds(points.as_flattened()), canvas.bounds()) {
        return;
    }

    if v_depth == 0 {
        draw_patch_pieces(canvas, patch, points, [0.0, 1.0], v_range, u_depth, colors);

        return;
    }

    let split = points.map(split_cubic);
    let low = split.map(|(low, _)| low);
    let high = split.map(|(_, high)| high);
    let middle = 0.5 * (v_range[0] + v_range[1]);
    let depth = [v_depth - 1, u_depth];

    draw_patch_rows(canvas, patch, low, [v_range[0], middle], depth, colors);
    draw_patch_rows(canvas, patch, high, [middle, v_range[1]], depth, colors);
}

fn draw_patch_pieces<const N: usize>(
    canvas: &mut Canvas,
    patch: &CanvasPatch<'_>,
    points: [[Point; 4]; 4],
    u_range: [f32; 2],
    v_range: [f32; 2],
    depth: u32,
    colors: &impl MeshColors<N>,
) {
    if !overlaps(bounds(points.as_flattened()), canvas.bounds()) {
        return;
    }

    if depth == 0 {
        let [u0, u1] = u_range;
        let [v0, v1] = v_range;
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

        return;
    }

    let (low, high) = split_columns(points);
    let middle = 0.5 * (u_range[0] + u_range[1]);

    draw_patch_pieces(
        canvas,
        patch,
        low,
        [u_range[0], middle],
        v_range,
        depth - 1,
        colors,
    );
    draw_patch_pieces(
        canvas,
        patch,
        high,
        [middle, u_range[1]],
        v_range,
        depth - 1,
        colors,
    );
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
