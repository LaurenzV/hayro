use skrifa::GlyphId;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct GlyphSimulator {
    // We simulate that Type1 glyphs have glyph IDs so we can handle them transparently
    // to OpenType fonts.
    glyph_to_string: RefCell<HashMap<GlyphId, String>>,
    string_to_glyph: RefCell<HashMap<String, GlyphId>>,
    glyph_counter: Cell<u32>,
}

impl GlyphSimulator {
    pub(crate) fn new() -> Self {
        let mut glyph_to_string = HashMap::new();
        glyph_to_string.insert(GlyphId::NOTDEF, "notdef".to_string());

        let mut string_to_glyph = HashMap::new();
        string_to_glyph.insert("notdef".to_string(), GlyphId::NOTDEF);

        Self {
            glyph_to_string: RefCell::new(glyph_to_string),
            string_to_glyph: RefCell::new(string_to_glyph),
            glyph_counter: Cell::new(1),
        }
    }

    pub(crate) fn string_to_glyph(&self, string: &str) -> GlyphId {
        if let Some(g) = self.string_to_glyph.borrow().get(string) {
            *g
        } else {
            let gid = GlyphId::new(self.glyph_counter.get());
            self.string_to_glyph
                .borrow_mut()
                .insert(string.to_string(), gid);
            self.glyph_to_string
                .borrow_mut()
                .insert(gid, string.to_string());

            self.glyph_counter.set(self.glyph_counter.get() + 1);

            gid
        }
    }

    pub(crate) fn glyph_to_string(&self, glyph: GlyphId) -> Option<String> {
        self.glyph_to_string
            .borrow()
            .get(&glyph)
            .map(|s| s.to_string())
    }
}
