use crate::{Renderer, convert_fill_rule};
use hayro_interpret::{DrawMode, DrawProps, FillRule, StrokeProps};
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use std::f64::consts::SQRT_2;

impl Renderer<'_> {
    pub(super) fn set_stroke_properties(&mut self, stroke_props: &StrokeProps, is_text: bool) {
        let threshold = if is_text { 0.25 } else { 1.0 };

        // Best-effort attempt to ensure a line width of at least 1.0, as required by the PDF
        // specification. If we are stroking text, we reduce the threshold as it will otherwise
        // lead to very bold-looking text at low resolutions.
        let min_factor = max_factor(self.ctx.transform());
        let mut line_width = stroke_props.line_width.max(0.01 / min_factor);
        let transformed_width = line_width * min_factor;

        // Only enforce line width if not inside of pattern or type 3 glyph.
        if transformed_width < threshold && !self.inside_pattern && !self.in_type3_glyph {
            line_width /= transformed_width;
            line_width *= threshold;
        }

        // At a device width of at most one pixel, round caps and joins are indistinguishable
        // from square caps and bevel joins, which are much cheaper to construct.
        let is_thin = transformed_width <= 1.0;
        let cap = match stroke_props.line_cap {
            kurbo::Cap::Round if is_thin => kurbo::Cap::Square,
            cap => cap,
        };
        let join = match stroke_props.line_join {
            kurbo::Join::Round if is_thin => kurbo::Join::Bevel,
            join => join,
        };

        let stroke = kurbo::Stroke {
            width: line_width as f64,
            join,
            miter_limit: stroke_props.miter_limit as f64,
            start_cap: cap,
            end_cap: cap,
            dash_pattern: stroke_props.dash_array.iter().map(|n| *n as f64).collect(),
            dash_offset: stroke_props.dash_offset as f64,
        };

        self.ctx.set_stroke(stroke);
    }

    pub(super) fn stroke_path(
        &mut self,
        path: &BezPath,
        props: DrawProps<'_>,
        stroke_props: &StrokeProps,
        is_text: bool,
    ) {
        self.apply_draw_props(&props);
        self.set_stroke_properties(stroke_props, is_text);

        let stroke = self.ctx.stroke();
        let extent = stroke.width / 2.0 * stroke.miter_limit.max(SQRT_2);
        let bbox = path.control_box().inflate(extent, extent);
        if self.misses_target(props.transform.transform_rect_bbox(bbox)) {
            return;
        }

        let clip_path = self.set_paint(&props.paint, || path.bounding_box(), true);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, FillRule::NonZero);
        }
        if let Some(outline) = single_segment_outline(path, self.ctx.stroke()) {
            self.ctx.set_fill_rule(convert_fill_rule(FillRule::NonZero));
            self.ctx.fill_path(&outline);
        } else {
            self.ctx.stroke_path(path);
        }
        if clip_path.is_some() {
            self.ctx.pop_clip();
        }
    }

    pub(super) fn fill_path(&mut self, path: &BezPath, props: DrawProps<'_>, fill_rule: FillRule) {
        self.ctx.set_fill_rule(convert_fill_rule(fill_rule));
        self.apply_draw_props(&props);

        let clip_path = self.set_paint(&props.paint, || path.bounding_box(), false);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, fill_rule);
        }

        self.ctx.fill_path(path);

        if clip_path.is_some() {
            self.ctx.pop_clip();
        }
    }

    pub(super) fn draw_path<'a>(
        &mut self,
        path: &BezPath,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        match draw_mode {
            DrawMode::Fill(f) => {
                Self::fill_path(self, path, props, *f);
            }
            DrawMode::Stroke(s) => {
                Self::stroke_path(self, path, props, s, false);
            }
            DrawMode::FillAndStroke(f, s) => {
                Self::fill_path(self, path, props.clone(), *f);
                Self::stroke_path(self, path, props, s, false);
            }
            DrawMode::Invisible => {}
        }
    }

    pub(super) fn draw_rect<'a>(
        &mut self,
        rect: &Rect,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        match draw_mode {
            DrawMode::Fill(fill_rule) => {
                self.ctx.set_fill_rule(convert_fill_rule(*fill_rule));
                self.apply_draw_props(&props);

                let clip_path = self.set_paint(&props.paint, || *rect, false);
                if let Some(clip_path) = clip_path.as_ref() {
                    self.push_clip_path_inner(clip_path, *fill_rule);
                }

                self.ctx.fill_rect(rect);

                if clip_path.is_some() {
                    self.ctx.pop_clip();
                }
            }
            DrawMode::Stroke(s) => {
                let path = rect.to_path(0.1);
                Self::stroke_path(self, &path, props, s, false);
            }
            DrawMode::FillAndStroke(fill_rule, stroke_props) => {
                self.draw_rect(rect, props.clone(), &DrawMode::Fill(*fill_rule));
                let path = rect.to_path(0.1);
                Self::stroke_path(self, &path, props, stroke_props, false);
            }
            DrawMode::Invisible => {}
        }
    }
}

pub(crate) fn max_factor(transform: &Affine) -> f32 {
    let scale_skew_transform = {
        let c = transform.as_coeffs();
        Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
    };

    let x_advance = scale_skew_transform * Point::new(1.0, 0.0);
    let y_advance = scale_skew_transform * Point::new(0.0, 1.0);

    x_advance
        .to_vec2()
        .length()
        .max(y_advance.to_vec2().length()) as f32
}

fn single_segment_outline(path: &BezPath, stroke: &kurbo::Stroke) -> Option<BezPath> {
    use kurbo::{Cap, PathEl};

    if !stroke.dash_pattern.is_empty() || stroke.start_cap != stroke.end_cap {
        return None;
    }

    let (p0, p1) = match path.elements() {
        [PathEl::MoveTo(p0), PathEl::LineTo(p1)] => (*p0, *p1),
        _ => return None,
    };

    let d = p1 - p0;
    let len = d.hypot();
    if len <= 0.0 || !len.is_finite() {
        return None;
    }

    let half = 0.5 * stroke.width;
    let dir = d / len;
    let n = kurbo::Vec2::new(-dir.y, dir.x) * half;
    let (a, b) = match stroke.start_cap {
        Cap::Butt => (p0, p1),
        Cap::Square => (p0 - dir * half, p1 + dir * half),
        Cap::Round => return None,
    };

    Some(BezPath::from_vec(vec![
        PathEl::MoveTo(a + n),
        PathEl::LineTo(b + n),
        PathEl::LineTo(b - n),
        PathEl::LineTo(a - n),
        PathEl::ClosePath,
    ]))
}
