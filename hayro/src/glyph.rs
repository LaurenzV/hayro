use crate::Renderer;
use hayro_interpret::font::Glyph;
use hayro_interpret::{CacheKey, DrawMode, DrawProps, FillRule, StrokeProps};
use kurbo::{Affine, BezPath};
use std::rc::Rc;

impl Renderer<'_> {
    fn fill_glyph<'a>(&mut self, glyph: &Glyph<'a>, props: DrawProps<'a>, glyph_transform: Affine) {
        match glyph {
            Glyph::Outline(o) => {
                let base_outline = self.cached_outline(o);
                let props = DrawProps {
                    transform: props.transform * glyph_transform,
                    ..props
                };

                self.fill_path(base_outline.as_ref(), props, FillRule::NonZero);
            }
            Glyph::Type3(s) => {
                self.in_type3_glyph = true;
                s.interpret(self, props.transform, glyph_transform, &props.paint);
                self.in_type3_glyph = false;
            }
        }
    }

    fn stroke_glyph<'a>(
        &mut self,
        glyph: &Glyph<'a>,
        props: DrawProps<'a>,
        glyph_transform: Affine,
        stroke_props: &StrokeProps,
    ) {
        match glyph {
            Glyph::Outline(o) => {
                let base_outline = self.cached_outline(o);

                self.stroke_path(
                    &(glyph_transform * base_outline.as_ref().clone()),
                    props,
                    stroke_props,
                    true,
                );
            }
            Glyph::Type3(s) => {
                s.interpret(self, props.transform, glyph_transform, &props.paint);
            }
        }
    }

    fn cached_outline(&self, glyph: &hayro_interpret::font::OutlineGlyph) -> Rc<BezPath> {
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

    pub(super) fn draw_glyph<'a>(
        &mut self,
        glyph: &Glyph<'a>,
        glyph_transform: Affine,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        match draw_mode {
            DrawMode::Fill(_) => {
                Self::fill_glyph(self, glyph, props, glyph_transform);
            }
            DrawMode::Stroke(s) => {
                Self::stroke_glyph(self, glyph, props, glyph_transform, s);
            }
            DrawMode::FillAndStroke(_, s) => {
                Self::fill_glyph(self, glyph, props.clone(), glyph_transform);
                Self::stroke_glyph(self, glyph, props, glyph_transform, s);
            }
            DrawMode::Invisible => {}
        }
    }
}
