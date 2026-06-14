use crate::SvgRenderer;
use crate::hash128;
use crate::path::BezPathExt;
use hayro_interpret::font::{Glyph, Type3Glyph};
use hayro_interpret::{CacheKey, DrawMode, DrawProps, Paint};
use kurbo::{Affine, BezPath, Shape};

pub(crate) struct CachedOutlineGlyph {
    path: BezPath,
}

#[derive(Clone)]
pub(crate) struct CachedType3Glyph<'a> {
    // TODO: Use Arc instead?
    glyph: Box<Type3Glyph<'a>>,
    transform: Affine,
    glyph_transform: Affine,
    paint: Paint<'a>,
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        glyph_transform: Affine,
        props: DrawProps<'a>,
        mode: &DrawMode,
    ) {
        if matches!(mode, DrawMode::Invisible) {
            return;
        }

        match glyph {
            Glyph::Outline(o) => {
                let outline = o.outline();
                let glyph_id = o.identifier().cache_key();
                let (cache_key, glyph_path, use_transform, paint_transform) = match mode {
                    DrawMode::Fill(_) => {
                        let transform = props.transform * glyph_transform;
                        (glyph_id, outline.clone(), transform, transform)
                    }
                    // TODO: Figure out how to better merge transform and glyph transform for stroked glyphs.
                    DrawMode::Stroke(_) | DrawMode::FillAndStroke(_, _) => (
                        hash128(&(glyph_id, glyph_transform.cache_key())),
                        glyph_transform * outline.clone(),
                        props.transform,
                        props.transform,
                    ),
                    DrawMode::Invisible => {
                        // We exited above.
                        unreachable!()
                    }
                };

                let id = self
                    .outline_glyphs
                    .insert_with(cache_key, || CachedOutlineGlyph { path: glyph_path });

                self.xml.start_element("use");
                self.xml
                    .write_attribute_fmt("xlink:href", format_args!("#{id}"));
                self.write_transform(use_transform);

                match mode {
                    DrawMode::Fill(_) => {
                        self.write_paint(
                            &props.paint,
                            || outline.bounding_box(),
                            paint_transform,
                            None,
                        );
                    }
                    DrawMode::Stroke(s) => {
                        self.write_stroke_properties(s);
                        self.write_paint(
                            &props.paint,
                            || outline.bounding_box(),
                            paint_transform,
                            Some(s),
                        );
                    }
                    DrawMode::FillAndStroke(_, s) => {
                        self.write_stroke_properties(s);
                        self.write_fill_and_stroke_paint(
                            &props.paint,
                            || outline.bounding_box(),
                            paint_transform,
                            s,
                        );
                    }
                    DrawMode::Invisible => {
                        // We exited above.
                        unreachable!()
                    }
                }
                self.xml.end_element();
            }
            Glyph::Type3(t) => {
                let cache_key = hash128(&(
                    t.cache_key(),
                    props.transform.cache_key(),
                    glyph_transform.cache_key(),
                    props.paint.cache_key(),
                ));

                if !self.type3_glyphs.contains(cache_key) {
                    self.with_dummy(|r| {
                        t.interpret(r, props.transform, glyph_transform, &props.paint);
                    });
                }

                // TODO: Apply transforms to group if possible
                let id = self
                    .type3_glyphs
                    .insert_with(cache_key, || CachedType3Glyph {
                        glyph: t.clone(),
                        transform: props.transform,
                        glyph_transform,
                        paint: props.paint.clone(),
                    });

                self.xml.start_element("use");
                self.xml
                    .write_attribute_fmt("xlink:href", format_args!("#{id}"));
                self.xml.end_element();
            }
        }
    }

    pub(crate) fn write_glyph_defs(&mut self) {
        if !self.outline_glyphs.is_empty() {
            self.xml.start_element("defs");
            self.xml.write_attribute("id", "outline-glyph");

            for (id, glyph) in self.outline_glyphs.iter() {
                self.xml.start_element("path");
                self.xml.write_attribute("id", &id);
                self.xml.write_attribute("d", &glyph.path.to_svg_f32());
                self.xml.end_element();
            }

            self.xml.end_element();
        }

        if !self.type3_glyphs.is_empty() {
            self.xml.start_element("defs");
            self.xml.write_attribute("id", "type3-glyph");
            let type3_glyphs = self.type3_glyphs.clone();

            for (id, glyph) in type3_glyphs.iter() {
                self.xml.start_element("g");
                self.xml.write_attribute("id", &id);
                glyph
                    .glyph
                    .interpret(self, glyph.transform, glyph.glyph_transform, &glyph.paint);
                self.xml.end_element();
            }

            self.xml.end_element();
        }
    }
}
