use crate::GlyphId;
use crate::program::content_hash;
use crate::program::pen::OutlinePath;
use crate::{FontData, UNITS_PER_EM};
use kurbo::BezPath;
use rustc_hash::FxHashMap;
use skrifa::raw::ps::cff::{CffFontRef, Subfont, charset::Charset};
use skrifa::raw::ps::string::Sid;
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, OnceLock};
use yoke::{Yoke, Yokeable};

#[derive(Clone)]
pub(crate) struct CffProgram(Arc<Inner>);

impl CffProgram {
    pub(crate) fn new(data: FontData) -> Option<Self> {
        let hash = content_hash(data.as_ref().as_ref(), 0);
        Self::with_hash(data, hash)
    }

    pub(crate) fn with_hash(data: FontData, hash: u128) -> Option<Self> {
        let yoke = Yoke::<CffYoke<'static>, FontData>::try_attach_to_cart(data, |data| {
            let font = CffFontRef::new(data.as_ref(), 0, None).map_err(|_| ())?;
            let charset = font.charset();
            let subfonts = (0..font.num_subfonts())
                .map(|index| font.subfont(index, &[]).ok())
                .collect::<Option<Vec<_>>>()
                .ok_or(())?;

            Ok::<_, ()>(CffYoke {
                font,
                charset,
                subfonts,
            })
        })
        .ok()?;

        Some(Self(Arc::new(Inner {
            hash,
            yoke,
            name_to_glyph: OnceLock::new(),
        })))
    }

    fn get(&self) -> &CffYoke<'_> {
        self.0.yoke.get()
    }

    pub(crate) fn content_hash(&self) -> u128 {
        self.0.hash
    }

    pub(crate) fn data(&self) -> &FontData {
        self.0.yoke.backing_cart()
    }

    pub(crate) fn is_cid(&self) -> bool {
        self.get().font.is_cid()
    }

    fn string(&self, sid: Sid) -> Option<&str> {
        std::str::from_utf8(self.get().font.string(sid)?).ok()
    }

    pub(crate) fn glyph_by_name(&self, name: &str) -> Option<GlyphId> {
        if self.is_cid() {
            return None;
        }

        self.0
            .name_to_glyph
            .get_or_init(|| {
                let Some(charset) = &self.get().charset else {
                    return FxHashMap::default();
                };

                let mut names = FxHashMap::default();
                for (gid, sid) in charset.iter() {
                    if let Some(name) = self.string(sid) {
                        names
                            .entry(name.to_string())
                            .or_insert(GlyphId::from_skrifa(gid));
                    }
                }

                names
            })
            .get(name)
            .copied()
    }

    fn glyph_name(&self, glyph: GlyphId) -> Option<&str> {
        if self.is_cid() {
            return None;
        }

        let sid = self
            .get()
            .charset
            .as_ref()?
            .string_id(glyph.to_skrifa())
            .ok()?;
        self.string(sid)
    }

    pub(crate) fn glyph_by_cid(&self, cid: u16) -> Option<GlyphId> {
        self.get()
            .charset
            .as_ref()?
            .glyph_id(Sid::new(cid))
            .ok()
            .map(GlyphId::from_skrifa)
    }

    pub(crate) fn builtin_encoding_glyph(&self, code: u8) -> Option<GlyphId> {
        self.get()
            .font
            .encoding()?
            .map(code)
            .map(GlyphId::from_skrifa)
    }

    pub(crate) fn builtin_encoding_glyph_name(&self, code: u8) -> Option<&str> {
        self.glyph_name(self.builtin_encoding_glyph(code)?)
    }

    pub(crate) fn outline(&self, glyph: GlyphId) -> BezPath {
        let font = &self.get().font;
        let mut path = OutlinePath::new();
        let Some(subfont) = font
            .subfont_index(glyph.to_skrifa())
            .and_then(|index| self.get().subfonts.get(index as usize))
        else {
            return BezPath::new();
        };

        let _ = font.draw(
            subfont,
            glyph.to_skrifa(),
            &[],
            Some(UNITS_PER_EM),
            &mut path,
        );

        path.take()
    }
}

impl Debug for CffProgram {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "CffProgram {{ .. }}")
    }
}

struct Inner {
    hash: u128,
    yoke: Yoke<CffYoke<'static>, FontData>,
    name_to_glyph: OnceLock<FxHashMap<String, GlyphId>>,
}

#[derive(Yokeable)]
struct CffYoke<'a> {
    font: CffFontRef<'a>,
    charset: Option<Charset<'a>>,
    subfonts: Vec<Subfont>,
}
