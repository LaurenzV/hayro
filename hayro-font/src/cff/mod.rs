//! A [Compact Font Format Table](
//! https://docs.microsoft.com/en-us/typography/opentype/spec/cff) implementation.

#![allow(clippy::upper_case_acronyms)]

// Useful links:
// http://wwwimages.adobe.com/content/dam/Adobe/en/devnet/font/pdfs/5176.CFF.pdf
// http://wwwimages.adobe.com/content/dam/Adobe/en/devnet/font/pdfs/5177.Type2.pdf
// https://github.com/opentypejs/opentype.js/blob/master/src/tables/cff.js

use super::GlyphId;
use crate::{Builder, OutlineBuilder, Rect, RectF};
use alloc::vec::Vec;
use skrifa::raw::{
    ps::{
        cff::{CffFontRef, Encoding, Subfont, charset::Charset},
        encoding::PredefinedEncoding,
        string::{STANDARD_STRINGS, Sid},
    },
    types::pen::NullPen,
};

/// A [Compact Font Format Table](
/// https://docs.microsoft.com/en-us/typography/opentype/spec/cff).
#[derive(Clone)]
pub struct Table<'a> {
    // The whole CFF table.
    // Used to resolve a local subroutine in a CID font.
    font: CffFontRef<'a>,
    charset: Option<Charset<'a>>,
    encoding: Option<Encoding<'a>>,
    subfonts: Vec<Option<Subfont>>,
}

impl<'a> Table<'a> {
    /// Parses a table from raw data.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        let font = CffFontRef::new(data, 0, None).ok()?;
        let encoding = font.encoding();
        let charset = encoding.as_ref().map(|e| e.charset().clone());
        let subfonts = (0..font.num_subfonts())
            .map(|i| font.subfont(i, &[]).ok())
            .collect::<Vec<_>>();
        Some(Self {
            font,
            charset,
            encoding,
            subfonts,
        })
    }

    /// Returns a total number of glyphs in the font.
    ///
    /// Never zero.
    #[inline]
    pub fn number_of_glyphs(&self) -> u16 {
        self.font.num_glyphs() as u16
    }

    /// Outlines a glyph.
    pub fn outline(&self, glyph_id: GlyphId, builder: &mut dyn OutlineBuilder) -> Option<Rect> {
        let mut rect_builder = Builder {
            builder,
            bbox: RectF::new(),
        };
        let subfont = self
            .subfonts
            .get(self.font.subfont_index(glyph_id)? as usize)?
            .as_ref()?;
        self.font
            .draw(subfont, glyph_id, &[], None, &mut rect_builder)
            .ok()?;
        Some(rect_builder.bbox.to_rect()?)
    }

    /// Resolves a Glyph ID for a code point.
    pub fn glyph_index(&self, code_point: u8) -> Option<GlyphId> {
        if self.font.is_cid() {
            None
        } else {
            let gid = if let Some(gid) = self.encoding.as_ref().and_then(|enc| enc.map(code_point))
            {
                gid
            } else {
                self.charset
                    .as_ref()?
                    .glyph_id(PredefinedEncoding::Standard.sid(code_point)?)
                    .ok()?
            };
            Some(gid)
        }
    }

    /// Returns a glyph width.
    pub fn glyph_width(&self, glyph_id: GlyphId) -> Option<u16> {
        if self.font.is_cid() {
            return None;
        }
        let subfont = self
            .subfonts
            .get(self.font.subfont_index(glyph_id)? as usize)?
            .as_ref()?;
        self.font
            .draw(subfont, glyph_id, &[], None, &mut NullPen)
            .ok()?
            .map(|w| w as u16)
    }

    /// Convert a CID to its correpsonding glyph id.
    pub fn glyph_index_by_cid(&self, cid: u16) -> Option<GlyphId> {
        if self.is_cid() {
            self.charset.as_ref()?.glyph_id(Sid::new(cid)).ok()
        } else {
            None
        }
    }

    /// Whether the font is a CID font.
    pub fn is_cid(&self) -> bool {
        self.font.is_cid()
    }

    /// Returns a glyph ID by a name.
    pub fn glyph_index_by_name(&self, name: &str) -> Option<GlyphId> {
        if self.is_cid() {
            None
        } else {
            // See PDFBOX-5987: We first check if there happens to be a custom SID
            // (even if it's a standard name), and only if not do we check
            // the standard names.
            let sid = if let Some(index) = self.font.strings().and_then(|strings| {
                (0..strings.count())
                    .position(|i| strings.get(i as usize).ok() == Some(name.as_bytes()))
            }) {
                Sid::new((STANDARD_STRINGS.len() + index) as u16)
            } else {
                STANDARD_STRINGS
                    .iter()
                    .position(|n| *n == name)
                    .map(|n| Sid::new(n as u16))?
            };
            self.charset.as_ref()?.glyph_id(sid).ok()
        }
    }

    /// Returns a glyph name.
    pub fn glyph_name(&self, glyph_id: GlyphId) -> Option<&'a str> {
        if self.font.is_cid() {
            None
        } else {
            let sid = self.charset.as_ref()?.string_id(glyph_id).ok()?;
            self.font
                .string(sid)
                .and_then(|s| core::str::from_utf8(s).ok())
        }
    }

    /// Returns the CID corresponding to a glyph ID.
    ///
    /// Returns `None` if this is not a `CIDFont`.
    pub fn glyph_cid(&self, glyph_id: GlyphId) -> Option<u16> {
        if self.font.is_cid() {
            self.charset
                .as_ref()?
                .string_id(glyph_id)
                .ok()
                .map(|sid| sid.to_u16())
        } else {
            None
        }
    }
}

impl core::fmt::Debug for Table<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Table {{ ... }}")
    }
}
