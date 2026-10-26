use crate::Renderer;
use hayro_interpret::pattern::TilingPattern;
use hayro_interpret::util::x_y_advances;
use kurbo::{Affine, Point, Rect};
use std::ops::{Range, RangeInclusive};
use std::sync::Arc;
use vello_cpu::peniko::{ImageQuality, ImageSampler};
use vello_cpu::{Image, ImageSource, Pixmap, RenderContext, peniko};

impl Renderer<'_> {
    pub(crate) fn tiling_paint(
        &self,
        t: &TilingPattern<'_>,
        area: Rect,
        is_stroke: bool,
    ) -> (Image, Affine) {
        let area = t.matrix.inverse().transform_rect_bbox(area);
        let (x_advance, y_advance) = x_y_advances(&t.matrix);
        let x = TileAxis::new(
            x_advance.length(),
            t.bbox.x0..t.bbox.x1,
            t.x_step,
            area.x0..area.x1,
        );
        let y = TileAxis::new(
            y_advance.length(),
            t.bbox.y0..t.bbox.y1,
            t.y_step,
            area.y0..area.y1,
        );

        let image = Image {
            image: ImageSource::Pixmap(Arc::new(self.render_tiling(t, &x, &y, is_stroke))),
            sampler: ImageSampler {
                x_extend: x.extend,
                y_extend: y.extend,
                quality: ImageQuality::Medium,
                alpha: t.opacity,
            },
        };
        let transform = t.matrix
            * Affine::translate((x.start, y.start))
            * Affine::scale_non_uniform(1.0 / x.scale, 1.0 / y.scale);

        (image, transform)
    }

    fn render_tiling(
        &self,
        t: &TilingPattern<'_>,
        x: &TileAxis,
        y: &TileAxis,
        is_stroke: bool,
    ) -> Pixmap {
        let to_texture =
            Affine::scale_non_uniform(x.scale, y.scale) * Affine::translate((-x.start, -y.start));
        let copies = x.copies.clone().count() * y.copies.clone().count();
        let mut ctx = self.child_context(x.size, y.size);

        if copies <= 1 || copies > MAX_COPIES {
            let offset = Affine::translate((
                f64::from(*x.copies.start()) * x.step,
                f64::from(*y.copies.start()) * y.step,
            ));
            self.render_cell(&mut ctx, t, to_texture * offset, is_stroke);
        } else {
            let cell = Arc::new({
                let width = (t.bbox.width() * x.scale).ceil().max(1.0) as u16;
                let height = (t.bbox.height() * y.scale).ceil().max(1.0) as u16;
                let mut ctx = self.child_context(width, height);
                let to_cell = Affine::scale_non_uniform(x.scale, y.scale)
                    * Affine::translate((-t.bbox.x0, -t.bbox.y0));
                self.render_cell(&mut ctx, t, to_cell, is_stroke);
                finish(ctx)
            });
            let cell_rect = Rect::new(0.0, 0.0, cell.width() as f64, cell.height() as f64);

            ctx.set_paint(Image {
                image: ImageSource::Pixmap(cell),
                sampler: ImageSampler {
                    x_extend: peniko::Extend::Pad,
                    y_extend: peniko::Extend::Pad,
                    quality: ImageQuality::Low,
                    alpha: 1.0,
                },
            });

            for j in y.copies.clone() {
                for i in x.copies.clone() {
                    let origin = to_texture
                        * Point::new(
                            t.bbox.x0 + f64::from(i) * x.step,
                            t.bbox.y0 + f64::from(j) * y.step,
                        );
                    ctx.set_transform(Affine::translate((origin.x.round(), origin.y.round())));
                    ctx.fill_rect(&cell_rect);
                }
            }
        }

        finish(ctx)
    }

    fn render_cell(
        &self,
        ctx: &mut RenderContext,
        t: &TilingPattern<'_>,
        transform: Affine,
        is_stroke: bool,
    ) {
        let mut renderer = Renderer::new(ctx, self.global);
        renderer.inside_pattern = true;
        t.interpret(&mut renderer, transform, is_stroke);
    }
}

fn finish(mut ctx: RenderContext) -> Pixmap {
    let mut pixmap = Pixmap::new(ctx.width(), ctx.height());
    ctx.flush();
    ctx.render(&mut pixmap, &mut vello_cpu::Resources::default());

    pixmap
}

const MAX_TEXTURE_SIZE: f64 = 8192.0;
const MAX_COPIES: usize = 64;

struct TileAxis {
    start: f64,
    scale: f64,
    size: u16,
    step: f64,
    copies: RangeInclusive<i32>,
    extend: peniko::Extend,
}

impl TileAxis {
    fn new(scale: f64, cell: Range<f64>, step: f32, area: Range<f64>) -> Self {
        let step = f64::from(step.abs());
        let width = cell.end - cell.start;
        let visible = (area.end - area.start).max(0.0);
        let repeat = visible >= step;
        let extent = if repeat { step } else { visible };

        let scale = scale.min(MAX_TEXTURE_SIZE / extent.max(width));
        let scale = (step * scale).round().max(1.0) / step;

        if repeat {
            Self {
                start: cell.start,
                scale,
                size: (step * scale).round() as u16,
                step,
                copies: (-width / step).floor() as i32 + 1..=0,
                extend: peniko::Extend::Repeat,
            }
        } else {
            let start = cell.start + ((area.start - cell.start) * scale).floor() / scale;

            Self {
                start,
                scale,
                size: ((area.end - start) * scale).ceil().max(1.0) as u16,
                step,
                copies: ((area.start - cell.end) / step).ceil() as i32
                    ..=((area.end - cell.start) / step).floor() as i32,
                extend: peniko::Extend::Pad,
            }
        }
    }
}
