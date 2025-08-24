use crate::SvgRenderer;
use crate::hash128;
use crate::path::BezPathExt;
use hayro_interpret::font::{Glyph, Type3Glyph};
use hayro_interpret::{CacheKey, GlyphDrawMode, Paint};
use kurbo::{Affine, BezPath};

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
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        mode: &GlyphDrawMode,
    ) {
        match glyph {
            Glyph::Outline(o) => {
                // TODO: Figure out how to better merge transform and glyph transform
                let outline = o.outline();
                let cache_key = hash128(&(o.identifier().cache_key(), glyph_transform.cache_key()));
                let id = self
                    .outline_glyphs
                    .insert_with(cache_key, || CachedOutlineGlyph {
                        path: glyph_transform * outline.clone(),
                    });

                self.xml.start_element("use");
                self.xml
                    .write_attribute_fmt("xlink:href", format_args!("#{id}"));
                self.write_transform(transform);

                match mode {
                    GlyphDrawMode::Fill => {
                        self.write_paint(paint, &outline, transform, false);
                    }
                    GlyphDrawMode::Stroke(_) => {
                        self.write_paint(paint, &outline, transform, true);
                    }
                }
                self.xml.end_element();
            }
            Glyph::Type3(t) => {
                let cache_key = hash128(&(
                    t.cache_key(),
                    transform.cache_key(),
                    glyph_transform.cache_key(),
                    paint.cache_key(),
                ));

                if !self.outline_glyphs.contains(cache_key) {
                    self.with_dummy(|r| {
                        t.interpret(r, transform, glyph_transform, paint);
                    });
                }

                // TODO: Apply transforms to group if possible
                let id = self
                    .type3_glyphs
                    .insert_with(cache_key, || CachedType3Glyph {
                        glyph: t.clone(),
                        transform,
                        glyph_transform,
                        paint: paint.clone(),
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
