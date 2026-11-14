use crate::cid::Type0Font;
use crate::program::FontProgram;
use crate::true_type::TrueTypeFont;
use crate::type1::Type1Font;
use hayro_cmap::BfString;
use std::rc::Rc;

/// The data and metadata of an outline font.
#[derive(Clone)]
pub struct OutlineFontData {
    /// The raw font bytes.
    pub data: crate::FontData,
    /// A key that identifies the font data.
    pub cache_key: u128,
    /// The PostScript name.
    pub postscript_name: Option<String>,
    /// The font weight.
    pub weight: Option<u32>,
    /// Whether the font is italic.
    pub is_italic: bool,
    /// Whether the font is serif.
    pub is_serif: bool,
    /// Whether the font is monospace.
    pub is_monospace: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum OutlineFont {
    Type1(Rc<Type1Font>),
    TrueType(Rc<TrueTypeFont>),
    Type0(Rc<Type0Font>),
}

impl OutlineFont {
    pub(crate) fn program(&self) -> FontProgram {
        match self {
            Self::Type1(t) => t.program(),
            Self::TrueType(t) => t.program(),
            Self::Type0(t) => t.program(),
        }
    }

    // Standard fonts have an empty `.notdef`, but their substitutes might not.
    pub(crate) fn hides_notdef(&self) -> bool {
        match self {
            Self::Type1(t) => t.is_standard(),
            Self::TrueType(t) => t.is_standard(),
            Self::Type0(_) => false,
        }
    }

    pub(crate) fn char_code_to_unicode(&self, char_code: u32) -> Option<BfString> {
        match self {
            Self::Type1(t) => t.char_code_to_unicode(char_code),
            Self::TrueType(t) => t.char_code_to_unicode(char_code),
            Self::Type0(t) => t.char_code_to_unicode(char_code),
        }
    }

    pub(crate) fn glyph_advance_width(&self, char_code: u32) -> Option<f32> {
        match self {
            Self::Type1(t) => t.glyph_width(char_code as u8),
            Self::TrueType(t) => Some(t.glyph_width(char_code as u8)),
            Self::Type0(t) => Some(t.code_advance(char_code).x as f32),
        }
    }

    pub(crate) fn font_data(&self) -> Option<OutlineFontData> {
        let cache_key = self.program().content_hash();

        match self {
            Self::Type1(_) => None,
            Self::TrueType(t) => Some(OutlineFontData {
                data: t.font_data()?,
                cache_key,
                postscript_name: t.postscript_name().map(|s| s.to_string()),
                weight: t.weight(),
                is_italic: t.is_italic(),
                is_serif: t.is_serif(),
                is_monospace: t.is_monospace(),
            }),
            Self::Type0(t) => Some(OutlineFontData {
                data: t.font_data()?,
                cache_key,
                postscript_name: t.postscript_name().map(|s| s.to_string()),
                weight: t.weight(),
                is_italic: t.is_italic(),
                is_serif: t.is_serif(),
                is_monospace: t.is_monospace(),
            }),
        }
    }
}
