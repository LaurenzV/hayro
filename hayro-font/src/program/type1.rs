use crate::GlyphId;
use crate::program::content_hash;
use crate::program::pen::OutlinePath;
use crate::{FontData, UNITS_PER_EM};
use kurbo::BezPath;
use rustc_hash::FxHashMap;
use skrifa::raw::ps::type1::Type1Font;
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub(crate) struct Type1Program(Arc<Inner>);

impl Type1Program {
    pub(crate) fn new(data: FontData) -> Option<Self> {
        let font = Type1Font::new(data.as_ref().as_ref()).ok()?;

        Some(Self(Arc::new(Inner {
            hash: content_hash(data.as_ref().as_ref(), 0),
            data,
            font,
            name_to_glyph: OnceLock::new(),
        })))
    }

    pub(crate) fn content_hash(&self) -> u128 {
        self.0.hash
    }

    pub(crate) fn data(&self) -> &FontData {
        &self.0.data
    }

    fn font(&self) -> &Type1Font {
        &self.0.font
    }

    pub(crate) fn glyph_by_name(&self, name: &str) -> Option<GlyphId> {
        self.0
            .name_to_glyph
            .get_or_init(|| {
                let mut names = FxHashMap::default();
                for (gid, name) in self.font().glyph_names() {
                    names
                        .entry(name.to_string())
                        .or_insert(GlyphId::from_skrifa(gid));
                }

                names
            })
            .get(name)
            .copied()
    }

    pub(crate) fn builtin_encoding_glyph(&self, code: u8) -> Option<GlyphId> {
        self.font().encoding()?.map(code).map(GlyphId::from_skrifa)
    }

    pub(crate) fn builtin_encoding_glyph_name(&self, code: u8) -> Option<&str> {
        self.font().encoding()?.glyph_name(code)
    }

    pub(crate) fn outline(&self, glyph: GlyphId) -> BezPath {
        let mut path = OutlinePath::new();
        let _ = self
            .font()
            .draw(glyph.to_skrifa(), Some(UNITS_PER_EM), &mut path);

        path.take()
    }
}

impl Debug for Type1Program {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Type1Program {{ .. }}")
    }
}

struct Inner {
    hash: u128,
    data: FontData,
    font: Type1Font,
    name_to_glyph: OnceLock<FxHashMap<String, GlyphId>>,
}
