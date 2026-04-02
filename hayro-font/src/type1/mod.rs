//! Reading Type1 fonts.

use super::OutlineBuilder;
use skrifa::{GlyphId, raw::ps::type1::Type1Font};

/// A Type1 font table.
pub struct Table {
    font: Type1Font,
}

impl Table {
    /// Parses a table from raw data.
    pub fn parse(data: &[u8]) -> Option<Self> {
        Some(Self {
            font: Type1Font::new(data).ok()?,
        })
    }

    /// Returns whether this is a `MultipleMaster` font.
    pub fn is_multiple_master(&self) -> bool {
        false
    }

    /// Given a glyph identifier in original order, returns the possibly
    /// remapped identifier.
    pub fn remapped_gid(&self, original_gid: GlyphId) -> GlyphId {
        self.font.remapped_gid(original_gid)
    }

    /// Outlines a glyph.
    pub fn outline(&self, gid: GlyphId, builder: &mut impl OutlineBuilder) -> Option<()> {
        self.font.draw(gid, None, builder).ok();
        Some(())
    }

    /// Outlines a glyph by name.
    pub fn outline_by_name(&self, string: &str, builder: &mut impl OutlineBuilder) -> Option<()> {
        let gid = self.font.glyph_names().find(|(_, name)| *name == string)?.0;
        self.font.draw(gid, None, builder).ok();
        Some(())
    }

    /// Return the glyph name of the code point.
    pub fn code_to_string(&self, code_point: u8) -> Option<&str> {
        self.font.encoding()?.glyph_name(code_point)
    }

    /// Returns the insertion index of a charstring by name.
    pub fn charstring_index(&self, name: &str) -> Option<u16> {
        Some(
            self.font
                .glyph_names()
                .find(|(_, string)| name == *string)?
                .0
                .to_u32() as u16,
        )
    }
}

impl core::fmt::Debug for Table {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Table {{ ... }}")
    }
}
