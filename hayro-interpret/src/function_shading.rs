//! Rasterization of function-based shadings.

use crate::canvas::{Canvas, bounds};
use crate::color::{color_error, lerp_color};
use crate::encode::EncodedShadingPattern;
use crate::shading::ShadingFunction;
use kurbo::{Affine, Point, Rect};

/// Initial cell size in pixels.
const FUNCTION_CELL: usize = 16;
/// Largest color error a cell may have before it is split further.
const FUNCTION_TOLERANCE: f32 = 0.01;
/// Size in pixels up to which cells are sampled at every pixel.
const FUNCTION_PIXEL_CELL: usize = 4;

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
        let points = (0..=columns)
            .map(|column| pixel_corner(corner(column, row)))
            .collect::<Vec<_>>();
        let mut colors = vec![[0.0; 4]; points.len()];
        sampler.sample(&points, &mut colors);

        colors
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
    fn sample(&self, points: &[Point], colors: &mut [[f32; 4]]) {
        const CHUNK: usize = 64;

        let mut inputs = [0.0; 2 * CHUNK];

        for (points, colors) in points.chunks(CHUNK).zip(colors.chunks_mut(CHUNK)) {
            for (input, point) in inputs.as_chunks_mut::<2>().0.iter_mut().zip(points) {
                let point = self.inverse * *point;
                *input = [point.x as f32, point.y as f32];
            }

            self.pattern
                .eval_many(self.function, &inputs[..2 * points.len()], colors);
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
    let corners_in_domain = [[0, 0], [size, 0], [0, size], [size, size]]
        .map(|[dx, dy]| sampler.inverse * Point::new((x + dx) as f64, (y + dy) as f64));

    if x >= canvas.width
        || y >= canvas.height
        || !bounds(&corners_in_domain).overlaps(sampler.domain)
    {
        return;
    }

    let inside = corners_in_domain
        .into_iter()
        .all(|p| sampler.domain.contains(p));

    if size <= FUNCTION_PIXEL_CELL {
        const MAX: usize = FUNCTION_PIXEL_CELL * FUNCTION_PIXEL_CELL;

        let mut indices = [0; MAX];
        let mut centers = [Point::ZERO; MAX];
        let mut colors = [[0.0; 4]; MAX];
        let mut count = 0;

        let (width, height) = (canvas.width, canvas.height);

        for_each_cell_pixel(
            [width, height],
            sampler,
            [x, y],
            size,
            inside,
            |index, center, _| {
                indices[count] = index;
                centers[count] = center;
                count += 1;
            },
        );

        sampler.sample(&centers[..count], &mut colors[..count]);

        for (index, color) in indices[..count].iter().zip(colors) {
            canvas.pixels[*index] = color;
        }

        return;
    }

    let half = size / 2;
    let [c00, c10, c01, c11] = corners;
    let mut samples = [[0.0; 4]; 5];
    sampler.sample(
        &[
            [x + half, y],
            [x, y + half],
            [x + half, y + half],
            [x + size, y + half],
            [x + half, y + size],
        ]
        .map(pixel_corner),
        &mut samples,
    );
    let [top, left, middle, right, bottom] = samples;

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
            let [c00, c10, c01, c11] = corners;
            let size = [canvas.width, canvas.height];

            for_each_cell_pixel(size, sampler, origin, half, inside, |index, _, [u, v]| {
                canvas.pixels[index] =
                    lerp_color(lerp_color(c00, c10, u), lerp_color(c01, c11, u), v);
            });
        }
    }
}

fn pixel_corner([x, y]: [usize; 2]) -> Point {
    Point::new(x as f64, y as f64)
}

fn for_each_cell_pixel(
    [width, height]: [usize; 2],
    sampler: &FunctionSampler<'_>,
    [x, y]: [usize; 2],
    size: usize,
    inside: bool,
    mut f: impl FnMut(usize, Point, [f32; 2]),
) {
    for row in y..(y + size).min(height) {
        for column in x..(x + size).min(width) {
            let center = Point::new(column as f64 + 0.5, row as f64 + 0.5);

            if inside || sampler.contains(center) {
                let position = [column - x, row - y].map(|o| (o as f32 + 0.5) / size as f32);
                f(row * width + column, center, position);
            }
        }
    }
}
