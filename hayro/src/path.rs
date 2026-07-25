use crate::{Renderer, convert_fill_rule};
use hayro_interpret::{DrawMode, DrawProps, FillRule, StrokeProps};
use kurbo::{Affine, BezPath, Point, Rect, Shape};

impl Renderer<'_> {
    pub(super) fn set_stroke_properties(&mut self, stroke_props: &StrokeProps, is_text: bool) {
        let threshold = if is_text { 0.25 } else { 1.0 };

        // Best-effort attempt to ensure a line width of at least 1.0, as required by the PDF
        // specification. If we are stroking text, we reduce the threshold as it will otherwise
        // lead to very bold-looking text at low resolutions.
        let min_factor = max_factor(self.ctx.transform());
        let mut line_width = stroke_props.line_width.max(0.01);
        let transformed_width = line_width * min_factor;

        // Only enforce line width if not inside of pattern or type 3 glyph.
        if transformed_width < threshold && !self.inside_pattern && !self.in_type3_glyph {
            line_width /= transformed_width;
            line_width *= threshold;
        }

        let stroke = kurbo::Stroke {
            width: line_width as f64,
            join: stroke_props.line_join,
            miter_limit: stroke_props.miter_limit as f64,
            start_cap: stroke_props.line_cap,
            end_cap: stroke_props.line_cap,
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

        let clip_path = self.set_paint(&props.paint, || path.bounding_box(), true);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, FillRule::NonZero);
        }
        self.ctx.stroke_path(path);
        if clip_path.is_some() {
            self.ctx.pop_clip_path();
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
            self.ctx.pop_clip_path();
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
                    self.ctx.pop_clip_path();
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
