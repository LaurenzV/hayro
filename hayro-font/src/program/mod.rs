use crate::FontData;
use crate::GlyphId;
use kurbo::BezPath;
use siphasher::sip128::{Hasher128, SipHasher13};
use std::hash::{Hash, Hasher};

mod cff;
mod open_type;
mod pen;
mod type1;

pub(crate) use cff::CffProgram;
pub(crate) use open_type::{CmapPlatform, CmapSubtable, OpenTypeProgram};
pub(crate) use type1::Type1Program;

#[derive(Clone, Debug)]
pub(crate) enum FontProgram {
    OpenType(OpenTypeProgram),
    Cff(CffProgram),
    Type1(Type1Program),
}

impl FontProgram {
    pub(crate) fn new(data: FontData, index: u32) -> Option<Self> {
        OpenTypeProgram::new(data.clone(), index)
            .map(Self::OpenType)
            .or_else(|| CffProgram::new(data.clone()).map(Self::Cff))
            .or_else(|| Type1Program::new(data).map(Self::Type1))
    }

    pub(crate) fn content_hash(&self) -> u128 {
        match self {
            Self::OpenType(p) => p.content_hash(),
            Self::Cff(p) => p.content_hash(),
            Self::Type1(p) => p.content_hash(),
        }
    }

    pub(crate) fn data(&self) -> &FontData {
        match self {
            Self::OpenType(p) => p.data(),
            Self::Cff(p) => p.data(),
            Self::Type1(p) => p.data(),
        }
    }

    pub(crate) fn glyph_by_name(&self, name: &str) -> Option<GlyphId> {
        match self {
            Self::OpenType(p) => p.glyph_by_name(name),
            Self::Cff(p) => p.glyph_by_name(name),
            Self::Type1(p) => p.glyph_by_name(name),
        }
    }

    pub(crate) fn builtin_encoding_glyph(&self, code: u8) -> Option<GlyphId> {
        match self {
            Self::OpenType(p) => p.builtin_encoding_glyph(code),
            Self::Cff(p) => p.builtin_encoding_glyph(code),
            Self::Type1(p) => p.builtin_encoding_glyph(code),
        }
    }

    pub(crate) fn builtin_encoding_glyph_name(&self, code: u8) -> Option<&str> {
        match self {
            Self::OpenType(p) => p.builtin_encoding_glyph_name(code),
            Self::Cff(p) => p.builtin_encoding_glyph_name(code),
            Self::Type1(p) => p.builtin_encoding_glyph_name(code),
        }
    }

    pub(crate) fn cmap_glyph(&self, c: char) -> Option<GlyphId> {
        match self {
            Self::OpenType(p) => p.cmap_glyph(c),
            Self::Cff(_) | Self::Type1(_) => None,
        }
    }

    pub(crate) fn advance_width(&self, glyph: GlyphId) -> Option<f32> {
        match self {
            Self::OpenType(p) => p.advance_width(glyph),
            Self::Cff(_) | Self::Type1(_) => None,
        }
    }

    pub(crate) fn weight(&self) -> Option<u32> {
        match self {
            Self::OpenType(p) => p.weight(),
            Self::Cff(_) | Self::Type1(_) => None,
        }
    }

    pub(crate) fn is_italic(&self) -> bool {
        match self {
            Self::OpenType(p) => p.is_italic(),
            Self::Cff(_) | Self::Type1(_) => false,
        }
    }

    pub(crate) fn is_monospace(&self) -> bool {
        match self {
            Self::OpenType(p) => p.is_monospace(),
            Self::Cff(_) | Self::Type1(_) => false,
        }
    }

    pub(crate) fn outline(&self, glyph: GlyphId) -> BezPath {
        match self {
            Self::OpenType(p) => p.outline(glyph),
            Self::Cff(p) => p.outline(glyph),
            Self::Type1(p) => p.outline(glyph),
        }
    }
}

impl PartialEq for FontProgram {
    fn eq(&self, other: &Self) -> bool {
        self.content_hash() == other.content_hash()
    }
}

impl Eq for FontProgram {}

impl Hash for FontProgram {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u128(self.content_hash());
    }
}

pub(crate) fn content_hash(data: &[u8], index: u32) -> u128 {
    let mut state = SipHasher13::new();
    (data, index).hash(&mut state);
    state.finish128().as_u128()
}
