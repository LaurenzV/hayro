//! A canvas that shadings are rasterized into.

use kurbo::{Point, Rect};

/// A grid of straight (non-premultiplied) RGBA pixels.
pub(crate) struct Canvas {
    pub(crate) width: usize,
    pub(crate) height: usize,
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

    pub(crate) fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f64, self.height as f64)
    }

    pub(crate) fn is_visible(&self, points: &[Point]) -> bool {
        bounds(points).overlaps(self.bounds())
    }

    /// Fill the pixel centers in the triangle; centers on an edge go to the triangle below or right.
    pub(crate) fn fill_triangle<const N: usize>(
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

pub(crate) fn bounds(points: &[Point]) -> Rect {
    points
        .iter()
        .fold(Rect::from_points(points[0], points[0]), |bounds, point| {
            bounds.union_pt(*point)
        })
}
