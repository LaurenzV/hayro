use crate::SvgRenderer;
use crate::hash128;
use crate::path::BezPathExt;
use hayro_interpret::font::Glyph;
use hayro_interpret::{CacheKey, GlyphDrawMode, Paint};
use kurbo::{Affine, BezPath};

pub(crate) struct CachedGlyph {
    path: BezPath,
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
                let outline = o.outline();
                let cache_key = hash128(&(o.identifier().cache_key(), glyph_transform.cache_key()));
                let id = self.glyphs.insert_with(cache_key, || CachedGlyph {
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
            Glyph::Type3(_) => {}
        }
    }

    pub(crate) fn write_glyph_defs(&mut self) {
        if self.glyphs.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "glyph");

        for (id, glyph) in self.glyphs.iter() {
            self.xml.start_element("path");
            self.xml.write_attribute("id", &id);
            self.xml.write_attribute("d", &glyph.path.to_svg_f32());
            self.xml.end_element();
        }

        self.xml.end_element();
    }
}
