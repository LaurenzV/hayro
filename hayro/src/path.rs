use crate::{Renderer, convert_fill_rule};
use hayro_interpret::{DrawMode, DrawProps, FillRule, StrokeProps};
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use std::f64::consts::SQRT_2;

const MIN_DASH_PERIOD: f32 = 0.1;

impl Renderer<'_> {
    pub(super) fn set_stroke_properties(&mut self, stroke_props: &StrokeProps, is_text: bool) {
        let threshold = if is_text { 0.25 } else { 1.0 };

        // Text gets a lower minimum width, since 1.0 makes it look bold at low resolutions.
        let min_factor = max_factor(self.ctx.transform());
        let mut line_width = stroke_props.line_width.max(0.01 / min_factor);
        let transformed_width = line_width * min_factor;

        if transformed_width < threshold && !self.inside_pattern && !self.in_type3_glyph {
            line_width /= transformed_width;
            line_width *= threshold;
        }

        // At most one pixel wide, round caps/joins look like square/bevel and are cheaper.
        let is_thin = transformed_width <= 1.0;
        let cap = match stroke_props.line_cap {
            kurbo::Cap::Round if is_thin => kurbo::Cap::Square,
            cap => cap,
        };
        let join = match stroke_props.line_join {
            kurbo::Join::Round if is_thin => kurbo::Join::Bevel,
            join => join,
        };

        let dash_period: f32 = stroke_props.dash_array.iter().map(|n| n.abs()).sum();
        let dash_pattern = if dash_period * min_factor >= MIN_DASH_PERIOD {
            stroke_props.dash_array.iter().map(|n| *n as f64).collect()
        } else {
            kurbo::Dashes::new()
        };

        let stroke = kurbo::Stroke {
            width: line_width as f64,
            join,
            miter_limit: stroke_props.miter_limit as f64,
            start_cap: cap,
            end_cap: cap,
            dash_pattern,
            dash_offset: stroke_props.dash_offset as f64,
        };

        self.ctx.set_stroke(stroke);
    }

    fn stroke_bounds(&self, bounds: Rect) -> Rect {
        let stroke = self.ctx.stroke();
        let factor = match stroke.join {
            kurbo::Join::Miter => stroke.miter_limit.abs().max(SQRT_2),
            kurbo::Join::Bevel | kurbo::Join::Round => {
                if matches!(stroke.start_cap, kurbo::Cap::Square)
                    || matches!(stroke.end_cap, kurbo::Cap::Square)
                {
                    SQRT_2
                } else {
                    1.0
                }
            }
        };
        let extent = stroke.width / 2.0 * factor;
        if extent.is_finite() {
            bounds.inflate(extent, extent)
        } else {
            bounds
        }
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

        let bbox = self.stroke_bounds(path.control_box());
        if self.misses_target(props.transform.transform_rect_bbox(bbox)) {
            return;
        }

        let clip_path =
            self.set_paint(&props.paint, |stroke| stroke_paint_bbox(path, stroke), true);
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

        let clip_path = self.set_paint(&props.paint, |_| path.bounding_box(), false);
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

                let clip_path = self.set_paint(&props.paint, |_| *rect, false);
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

pub(super) fn stroke_paint_bbox(path: &BezPath, stroke: &kurbo::Stroke) -> Rect {
    use kurbo::{PathEl, Vec2};

    let mut bbox = path.bounding_box();
    if stroke.join != kurbo::Join::Miter
        || !stroke.miter_limit.is_finite()
        || stroke.miter_limit.abs() <= 1.0
    {
        return bbox;
    }
    let mut join = |vertex: Point, incoming: Vec2, outgoing: Vec2| {
        let in_len = incoming.hypot();
        let out_len = outgoing.hypot();
        if !in_len.is_finite() || !out_len.is_finite() || in_len == 0. || out_len == 0. {
            return;
        }
        let u = incoming / in_len;
        let v = outgoing / out_len;
        let cross = u.cross(v);
        let sum = 1. + u.dot(v);
        if cross != 0. && sum > 0. && 2. < sum * stroke.miter_limit.powi(2) {
            let normal = Vec2::new(-(u.y + v.y), u.x + v.x);
            let tip = vertex + normal * (-cross.signum() * 0.5 * stroke.width / sum);
            if tip.is_finite() {
                bbox = bbox.union_pt(tip);
            }
        }
    };
    // Kurbo's PathSeg::tangents treats near-coincident controls as coincident.
    let tangent = |a: Point, b: Point, c: Point, d: Point| {
        const MIN_TANGENT_SQUARED: f64 = 1e-12;
        [b - a, c - a]
            .into_iter()
            .find(|v| v.hypot2() > MIN_TANGENT_SQUARED)
            .unwrap_or(d - a)
    };
    let mut point = Point::ORIGIN;
    let mut start = point;
    let mut first = None;
    let mut last = None;
    for el in path.iter() {
        let (end, outgoing, incoming) = match el {
            PathEl::MoveTo(p) => {
                start = p;
                point = p;
                first = None;
                last = None;
                continue;
            }
            PathEl::LineTo(p) => (p, p - point, p - point),
            PathEl::QuadTo(b, p) => (p, tangent(point, b, p, p), -tangent(p, b, point, point)),
            PathEl::CurveTo(b, c, p) => (p, tangent(point, b, c, p), -tangent(p, c, b, point)),
            PathEl::ClosePath => (start, start - point, start - point),
        };
        if outgoing != Vec2::ZERO && incoming != Vec2::ZERO {
            if let Some(prev) = last {
                join(point, prev, outgoing);
            }
            first.get_or_insert(outgoing);
            last = Some(incoming);
        }
        point = end;
        if el == PathEl::ClosePath {
            if let (Some(prev), Some(next)) = (last, first) {
                join(start, prev, next);
            }
            first = None;
            last = None;
        }
    }
    bbox
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
