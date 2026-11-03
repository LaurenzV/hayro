use crate::{Renderer, convert_fill_rule};
use hayro_interpret::font::{Glyph, GlyphRun, OutlineGlyph};
use hayro_interpret::hayro_cmap::WritingMode;
use hayro_interpret::{DrawMode, DrawProps, FillRule, Paint, StrokeProps};
use kurbo::{Affine, BezPath, Rect, Shape};
use std::rc::Rc;
use vello_cpu::color::AlphaColor;

const MIN_HINTING_PPEM: f64 = 4.0;
const MAX_HINTING_PPEM: f64 = 128.0;

impl Renderer<'_> {
    fn fill_glyph<'a>(
        &mut self,
        glyph: &Glyph<'a>,
        props: DrawProps<'a>,
        transform: Affine,
        hint: bool,
    ) {
        match glyph {
            Glyph::Outline(glyph) => {
                let id = glyph.identifier().cache_key();
                let (outline, transform, _) =
                    self.fill_outline(glyph, id, props.transform * transform, hint);
                let props = DrawProps { transform, ..props };

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
                let outline = self.cached_outline(glyph, glyph.identifier().cache_key());
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

    // TODO: Also hint stroked and clipping text, which can be off from hinted fills.
    // TODO: Don't hint text that is filled with a pattern.
    fn hint(&self, glyph_run: &GlyphRun<'_, '_>) -> bool {
        self.global.hinting && glyph_run.writing_mode() == WritingMode::Horizontal
    }

    fn fill_outline(
        &self,
        glyph: &OutlineGlyph,
        id: u128,
        transform: Affine,
        hint: bool,
    ) -> (Rc<BezPath>, Affine, bool) {
        let [a, b, c, d, e, f] = transform.as_coeffs();
        let ppem = d.abs() * 1000.0;

        if hint && b == 0.0 && (MIN_HINTING_PPEM..=MAX_HINTING_PPEM).contains(&ppem) {
            let ppem = ppem as f32;

            if let Some(hinted) = self.cached_hinted_outline(glyph, id, ppem) {
                return (hinted, Affine::new([a, b, c, d, e, f.round()]), true);
            }
        }

        (self.cached_outline(glyph, id), transform, false)
    }

    fn cached_hinted_outline(
        &self,
        glyph: &OutlineGlyph,
        id: u128,
        ppem: f32,
    ) -> Option<Rc<BezPath>> {
        self.cached_path(id, ppem, || {
            glyph.hinted_outline(ppem, &self.global.hinting_cache)
        })
    }

    fn cached_outline(&self, glyph: &OutlineGlyph, id: u128) -> Rc<BezPath> {
        self.cached_path(id, 0.0, || Some(glyph.outline()))
            .unwrap_or_default()
    }

    // Unhinted outlines are stored with a size of 0.
    fn cached_path(
        &self,
        id: u128,
        ppem: f32,
        path: impl FnOnce() -> Option<BezPath>,
    ) -> Option<Rc<BezPath>> {
        let key = (id, ppem.to_bits());

        if let Some(path) = self.global.outline_cache.borrow().get(&key) {
            return path.clone();
        }

        let path = path().map(Rc::new);
        self.global
            .outline_cache
            .borrow_mut()
            .insert(key, path.clone());
        path
    }

    fn transformed_fill_outlines(
        &self,
        glyph_run: &GlyphRun<'_, '_>,
        transform: Affine,
    ) -> Vec<BezPath> {
        let inverse = transform.inverse();

        glyph_run
            .glyphs()
            .iter()
            .filter_map(|glyph| {
                let Glyph::Outline(outline) = &**glyph else {
                    return None;
                };

                let (path, glyph_transform, _) = self.fill_outline(
                    outline,
                    outline.identifier().cache_key(),
                    transform * glyph.transform(),
                    self.hint(glyph_run),
                );
                Some(inverse * glyph_transform * path.as_ref().clone())
            })
            .collect()
    }

    fn transformed_outlines(&self, glyph_run: &GlyphRun<'_, '_>) -> Vec<BezPath> {
        glyph_run
            .glyphs()
            .iter()
            .filter_map(|glyph| {
                let Glyph::Outline(outline) = &**glyph else {
                    return None;
                };

                let outline = self.cached_outline(outline, outline.identifier().cache_key());
                Some(glyph.transform() * outline.as_ref().clone())
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
            self.ctx.pop_clip();
        }
    }

    fn fill_outline_run(&mut self, glyph_run: &GlyphRun<'_, '_>, props: &DrawProps<'_>) {
        match &props.paint {
            Paint::Color(color) => {
                let color = color.to_rgba().to_rgba8();
                let color = AlphaColor::from_rgba8(color[0], color[1], color[2], color[3]);
                let hint = self.hint(glyph_run);
                let use_cache = self.global.glyph_caching
                    && self.global.aliasing_threshold.is_none()
                    && glyph_run.writing_mode() == WritingMode::Horizontal;
                let clip_path = self.begin_fill_run(props, Rect::ZERO);
                for glyph in glyph_run.glyphs() {
                    let Glyph::Outline(outline) = &**glyph else {
                        continue;
                    };

                    let id = outline.identifier().cache_key();
                    let (path, transform, hinted) =
                        self.fill_outline(outline, id, props.transform * glyph.transform(), hint);

                    if !use_cache || !self.fill_cached_glyph(id, hinted, &path, transform, color) {
                        self.ctx.set_paint(color);
                        self.ctx.set_transform(transform);
                        self.ctx.fill_path(path.as_ref());
                    }
                }
                self.finish_run(clip_path);
            }
            Paint::Pattern(_) => {
                let outlines = self.transformed_fill_outlines(glyph_run, props.transform);
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

                    let outline = self.cached_outline(outline, outline.identifier().cache_key());
                    let outline = glyph.transform() * outline.as_ref().clone();
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
        let hint = self.hint(glyph_run);
        for glyph in glyph_run.glyphs() {
            let transform = glyph.transform();
            match draw_mode {
                DrawMode::Fill(_) => {
                    Self::fill_glyph(self, glyph, props.clone(), transform, hint);
                }
                DrawMode::Stroke(stroke) => {
                    Self::stroke_glyph(self, glyph, props.clone(), transform, stroke);
                }
                DrawMode::FillAndStroke(_, stroke) => {
                    Self::fill_glyph(self, glyph, props.clone(), transform, hint);
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
