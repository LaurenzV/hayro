//! Rasterization of function-based shadings.

use crate::canvas::{Canvas, bounds};
use crate::encode::EncodedShadingPattern;
use crate::function::MAX_COMPONENTS;
use crate::gradient::{color_error, lerp_color};
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
