use crate::{Renderer, convert_fill_rule};
use hayro_interpret::font::{Glyph, GlyphRun, OutlineGlyph};
use hayro_interpret::{CacheKey, DrawMode, DrawProps, FillRule, Paint, StrokeProps};
use kurbo::{Affine, BezPath, Rect, Shape};
use std::rc::Rc;

impl Renderer<'_> {
    fn fill_glyph<'a>(&mut self, glyph: &Glyph<'a>, props: DrawProps<'a>, transform: Affine) {
        match glyph {
            Glyph::Outline(glyph) => {
                let outline = self.cached_outline(glyph);
                let props = DrawProps {
                    transform: props.transform * transform,
                    ..props
                };

                self.fill_path(outline.as_ref(), props, FillRule::NonZero);
            }
            Glyph::Type3(glyph) => {
                self.in_type3_glyph = true;
                glyph.interpret(self, props.transform, transform, &props.paint);
                self.in_type3_glyph = false;
            }
        }
    }

    fn stroke_glyph<'a>(
        &mut self,
        glyph: &Glyph<'a>,
        props: DrawProps<'a>,
        transform: Affine,
        stroke_props: &StrokeProps,
    ) {
        match glyph {
            Glyph::Outline(glyph) => {
                let outline = self.cached_outline(glyph);
                self.stroke_path(
                    &(transform * outline.as_ref().clone()),
                    props,
                    stroke_props,
                    true,
                );
            }
            Glyph::Type3(glyph) => {
                glyph.interpret(self, props.transform, transform, &props.paint);
            }
        }
    }

    fn cached_outline(&self, glyph: &OutlineGlyph) -> Rc<BezPath> {
        let id = glyph.identifier().cache_key();

        if let Some(path) = self.global.outline_cache.borrow().get(&id) {
            return path.clone();
        }

        let path = Rc::new(glyph.outline());
        self.global
            .outline_cache
            .borrow_mut()
            .insert(id, path.clone());
        path
    }

    fn transformed_outlines(&self, glyph_run: &GlyphRun<'_, '_>) -> Vec<BezPath> {
        glyph_run
            .glyphs()
            .iter()
            .filter_map(|glyph| {
                let Glyph::Outline(outline) = &**glyph else {
                    return None;
                };

                Some(glyph.transform() * self.cached_outline(outline).as_ref().clone())
            })
            .collect()
    }

    fn outline_bbox(outlines: &[BezPath]) -> Rect {
        outlines
            .iter()
            .filter(|outline| !outline.elements().is_empty())
            .map(Shape::bounding_box)
            .reduce(|bbox, glyph_bbox| bbox.union(glyph_bbox))
            .unwrap_or(Rect::ZERO)
    }

    fn begin_fill_run(&mut self, props: &DrawProps<'_>, bbox: Rect) -> Option<BezPath> {
        self.ctx.set_fill_rule(convert_fill_rule(FillRule::NonZero));
        self.apply_draw_props(props);

        let clip_path = self.set_paint(&props.paint, || bbox, false);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, FillRule::NonZero);
        }

        clip_path
    }

    fn begin_stroke_run(
        &mut self,
        props: &DrawProps<'_>,
        stroke_props: &StrokeProps,
        bbox: Rect,
    ) -> Option<BezPath> {
        self.apply_draw_props(props);
        self.set_stroke_properties(stroke_props, true);

        let clip_path = self.set_paint(&props.paint, || bbox, true);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, FillRule::NonZero);
        }

        clip_path
    }

    fn finish_run(&mut self, clip_path: Option<BezPath>) {
        if clip_path.is_some() {
            self.ctx.pop_clip_path();
        }
    }

    fn fill_outline_run(&mut self, glyph_run: &GlyphRun<'_, '_>, props: &DrawProps<'_>) {
        match &props.paint {
            Paint::Color(_) => {
                let clip_path = self.begin_fill_run(props, Rect::ZERO);
                for glyph in glyph_run.glyphs() {
                    let Glyph::Outline(outline) = &**glyph else {
                        continue;
                    };

                    self.ctx.set_transform(props.transform * glyph.transform());
                    self.ctx.fill_path(self.cached_outline(outline).as_ref());
                }
                self.finish_run(clip_path);
            }
            Paint::Pattern(_) => {
                let outlines = self.transformed_outlines(glyph_run);
                let clip_path = self.begin_fill_run(props, Self::outline_bbox(&outlines));
                for outline in &outlines {
                    self.ctx.fill_path(outline);
                }
                self.finish_run(clip_path);
            }
        }
    }

    fn stroke_outline_run(
        &mut self,
        glyph_run: &GlyphRun<'_, '_>,
        props: &DrawProps<'_>,
        stroke_props: &StrokeProps,
    ) {
        match &props.paint {
            Paint::Color(_) => {
                let clip_path = self.begin_stroke_run(props, stroke_props, Rect::ZERO);
                for glyph in glyph_run.glyphs() {
                    let Glyph::Outline(outline) = &**glyph else {
                        continue;
                    };

                    let outline = glyph.transform() * self.cached_outline(outline).as_ref().clone();
                    self.ctx.stroke_path(&outline);
                }
                self.finish_run(clip_path);
            }
            Paint::Pattern(_) => {
                let outlines = self.transformed_outlines(glyph_run);
                let clip_path =
                    self.begin_stroke_run(props, stroke_props, Self::outline_bbox(&outlines));
                for outline in &outlines {
                    self.ctx.stroke_path(outline);
                }
                self.finish_run(clip_path);
            }
        }
    }

    fn draw_glyphs_individually<'a>(
        &mut self,
        glyph_run: &GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        for glyph in glyph_run.glyphs() {
            let transform = glyph.transform();
            match draw_mode {
                DrawMode::Fill(_) => {
                    Self::fill_glyph(self, glyph, props.clone(), transform);
                }
                DrawMode::Stroke(stroke) => {
                    Self::stroke_glyph(self, glyph, props.clone(), transform, stroke);
                }
                DrawMode::FillAndStroke(_, stroke) => {
                    Self::fill_glyph(self, glyph, props.clone(), transform);
                    Self::stroke_glyph(self, glyph, props.clone(), transform, stroke);
                }
                DrawMode::Invisible => {}
            }
        }
    }

    pub(super) fn draw_glyph_run<'a>(
        &mut self,
        glyph_run: &GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        let outlines_only = glyph_run
            .glyphs()
            .iter()
            .all(|glyph| matches!(&**glyph, Glyph::Outline(_)));

        if !outlines_only {
            self.draw_glyphs_individually(glyph_run, props, draw_mode);

            return;
        }

        match draw_mode {
            DrawMode::Fill(_) => self.fill_outline_run(glyph_run, &props),
            DrawMode::Stroke(stroke) => self.stroke_outline_run(glyph_run, &props, stroke),
            DrawMode::FillAndStroke(_, stroke) => {
                self.fill_outline_run(glyph_run, &props);
                self.stroke_outline_run(glyph_run, &props, stroke);
            }
            DrawMode::Invisible => {}
        }
    }
}
