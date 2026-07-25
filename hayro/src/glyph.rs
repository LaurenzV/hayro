use crate::Renderer;
use hayro_interpret::font::{Glyph, GlyphRun, OutlineGlyph};
use hayro_interpret::{CacheKey, DrawMode, DrawProps, FillRule, StrokeProps};
use kurbo::{Affine, BezPath};
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

    pub(super) fn draw_glyph_run<'a>(
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
}
