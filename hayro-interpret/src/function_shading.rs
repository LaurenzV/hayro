use crate::canvas::{Canvas, bounds};
use crate::color::{color_error, lerp_color};
use crate::encode::EncodedShadingPattern;
use crate::shading::ShadingFunction;
use kurbo::{Affine, Point, Rect};

const FUNCTION_CELL: usize = 16;
const FUNCTION_TOLERANCE: f32 = 0.01;
const FUNCTION_PIXEL_CELL: usize = 4;

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
    fn sample(&self, [x, y]: [usize; 2]) -> [f32; 4] {
        self.sample_at(Point::new(x as f64, y as f64))
    }

    fn sample_at(&self, point: Point) -> [f32; 4] {
        let point = self.inverse * point;
        self.pattern
            .eval(self.function, &[point.x as f32, point.y as f32])
    }

    fn contains(&self, point: Point) -> bool {
        self.domain.contains(self.inverse * point)
    }
}

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
        fill_cell(canvas, sampler, [x, y], size, inside, |center, _| {
            sampler.sample_at(center)
        });

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
            let [c00, c10, c01, c11] = corners;
            fill_cell(canvas, sampler, origin, half, inside, |_, [u, v]| {
                lerp_color(lerp_color(c00, c10, u), lerp_color(c01, c11, u), v)
            });
        }
    }
}

fn fill_cell(
    canvas: &mut Canvas,
    sampler: &FunctionSampler<'_>,
    [x, y]: [usize; 2],
    size: usize,
    inside: bool,
    color: impl Fn(Point, [f32; 2]) -> [f32; 4],
) {
    for row in y..(y + size).min(canvas.height) {
        for column in x..(x + size).min(canvas.width) {
            let center = Point::new(column as f64 + 0.5, row as f64 + 0.5);

            if inside || sampler.contains(center) {
                let position = [column - x, row - y].map(|o| (o as f32 + 0.5) / size as f32);
                canvas.pixels[row * canvas.width + column] = color(center, position);
            }
        }
    }
}
