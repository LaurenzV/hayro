use crate::font::blob::{CffFontBlob, Type1FontBlob};
use crate::font::standard_font::{StandardFont, StandardKind, select_standard_font};
use crate::font::true_type::{Width, read_encoding, read_widths};
use crate::font::{
    Encoding, FallbackFontQuery, glyph_name_to_unicode, normalized_glyph_name, read_to_unicode,
};
use crate::{CMapResolverFn, CacheKey, FontResolverFn};
use hayro_cmap::{BfString, CMap};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{FONT_DESC, FONT_FILE, FONT_FILE3};
use kurbo::BezPath;
use skrifa::GlyphId;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Type1Font(u128, Kind, Option<CMap>);

impl Type1Font {
    pub(crate) fn new(
        dict: &Dict<'_>,
        resolver: &FontResolverFn,
        cmap_resolver: &CMapResolverFn,
    ) -> Option<Self> {
        let cache_key = dict.cache_key();

        let to_unicode = read_to_unicode(dict, cmap_resolver);

        let fallback = || {
            // TODO: Actually use fallback fonts
            let fallback_query = FallbackFontQuery::new(dict);
            let standard_font = fallback_query.pick_standard_font();

            warn!(
                "unable to load font {}, falling back to {}",
                fallback_query
                    .post_script_name
                    .unwrap_or("(no name)".to_string()),
                standard_font.as_str()
            );

            Some(Self(
                cache_key,
                Kind::Standard(StandardKind::new_with_standard(
                    dict,
                    standard_font,
                    true,
                    resolver,
                )?),
                to_unicode.clone(),
            ))
        };

        let inner = if is_cff(dict) {
            if let Some(cff) = CffKind::new(dict) {
                Self(cache_key, Kind::Cff(cff), to_unicode)
            } else {
                return fallback();
            }
        } else if is_type1(dict) {
            if let Some(f) = Type1Kind::new(dict) {
                Self(cache_key, Kind::Type1(f), to_unicode)
            } else {
                return fallback();
            }
        } else if let Some(standard) = StandardKind::new(dict, resolver) {
            Self(cache_key, Kind::Standard(standard), to_unicode)
        } else {
            return fallback();
        };

        Some(inner)
    }

    pub(crate) fn new_standard(font: StandardFont, resolver: &FontResolverFn) -> Option<Self> {
        let dict = Dict::default();
        let standard = StandardKind::new_with_standard(&dict, font, true, resolver)?;

        Some(Self(0, Kind::Standard(standard), None))
    }

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
        match &self.1 {
            Kind::Standard(s) => s.map_code(code),
            Kind::Type1(s) => s.map_code(code),
            Kind::Cff(c) => c.map_code(code),
        }
    }

    pub(crate) fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        match &self.1 {
            Kind::Standard(s) => s.outline_glyph(glyph),
            Kind::Cff(c) => c.outline_glyph(glyph),
            Kind::Type1(t) => t.outline_glyph(glyph),
        }
    }

    pub(crate) fn glyph_width(&self, code: u8) -> Option<f32> {
        match &self.1 {
            Kind::Standard(s) => s.glyph_width(code),
            Kind::Cff(c) => c.glyph_width(code),
            Kind::Type1(t) => t.glyph_width(code),
        }
    }

    pub(crate) fn char_code_to_unicode(&self, char_code: u32) -> Option<BfString> {
        if let Some(to_unicode) = &self.2
            && let Some(c) = to_unicode.lookup_bf_string(char_code)
        {
            // Skip null character mappings and fall back to glyph name
            // lookup. Some PDFs have incorrect ToUnicode mappings that map
            // to U+0000.
            if c != BfString::Char('\0') {
                return Some(c);
            }
        }

        let code = char_code as u8;
        match &self.1 {
            Kind::Standard(s) => s.char_code_to_unicode(code).map(BfString::Char),
            Kind::Cff(c) => c.char_code_to_unicode(code).map(BfString::Char),
            Kind::Type1(t) => t.char_code_to_unicode(code).map(BfString::Char),
        }
    }
}

impl CacheKey for Type1Font {
    fn cache_key(&self) -> u128 {
        self.0
    }
}

#[derive(Debug)]
enum Kind {
    Standard(StandardKind),
    Cff(CffKind),
    Type1(Type1Kind),
}

fn is_cff(dict: &Dict<'_>) -> bool {
    dict.get::<Dict<'_>>(FONT_DESC)
        .map(|dict| dict.contains_key(FONT_FILE3))
        .unwrap_or(false)
}

fn is_type1(dict: &Dict<'_>) -> bool {
    dict.get::<Dict<'_>>(FONT_DESC)
        .map(|dict| dict.contains_key(FONT_FILE))
        .unwrap_or(false)
}

#[derive(Debug)]
struct Type1Kind {
    font: Type1FontBlob,
    encoding: Encoding,
    widths: Vec<Width>,
    missing_width: f32,
    encodings: HashMap<u8, String>,
    name_to_gid: HashMap<String, GlyphId>,
    standard_font: Option<StandardFont>,
}

impl Type1Kind {
    fn new(dict: &Dict<'_>) -> Option<Self> {
        let descriptor = dict.get::<Dict<'_>>(FONT_DESC)?;
        let data = descriptor.get::<Stream<'_>>(FONT_FILE)?;
        let font = Type1FontBlob::new(Arc::new(data.decoded().ok()?.to_vec()))?;

        let (encoding, encodings) = read_encoding(dict);
        let (widths, missing_width) = read_widths(dict, &descriptor)?;
        let standard_font = select_standard_font(dict, &descriptor).map(|(f, _)| f);

        let name_to_gid: HashMap<String, GlyphId> = font
            .table()
            .glyph_names()
            .map(|(gid, name)| (name.to_string(), gid))
            .collect();

        Some(Self {
            font,
            encoding,
            widths,
            missing_width,
            encodings,
            name_to_gid,
            standard_font,
        })
    }

    fn name_to_glyph(&self, name: &str) -> Option<GlyphId> {
        self.name_to_gid.get(name).copied()
    }

    fn map_code(&self, code: u8) -> GlyphId {
        if let Some(entry) = self.encodings.get(&code) {
            self.name_to_glyph(entry)
        } else {
            match self.encoding {
                Encoding::BuiltIn => self.font.table().encoding().and_then(|e| e.map(code)),
                _ => self
                    .encoding
                    .map_code(code)
                    .and_then(|name| self.name_to_glyph(name)),
            }
        }
        .unwrap_or(GlyphId::NOTDEF)
    }

    fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        self.font.outline_glyph(glyph)
    }

    fn code_to_ps_name(&self, code: u8) -> Option<&str> {
        self.encodings
            .get(&code)
            .map(String::as_str)
            .or_else(|| match self.encoding {
                Encoding::BuiltIn => self
                    .font
                    .table()
                    .encoding()
                    .and_then(|e| e.glyph_name(code)),
                _ => self.encoding.map_code(code),
            })
    }

    fn glyph_width(&self, code: u8) -> Option<f32> {
        match self.widths.get(code as usize).copied() {
            Some(Width::Value(w)) => Some(w),
            Some(Width::Missing) => Some(self.missing_width),
            _ => {
                // If font looks like a standard font, get the width from there.
                let sf = self.standard_font?;
                self.code_to_ps_name(code)
                    .and_then(|name| sf.get_width(name))
            }
        }
    }

    fn char_code_to_unicode(&self, code: u8) -> Option<char> {
        self.code_to_ps_name(code).and_then(glyph_name_to_unicode)
    }
}

#[derive(Debug)]
struct CffKind {
    font: CffFontBlob,
    encoding: Encoding,
    widths: Vec<Width>,
    missing_width: f32,
    encodings: HashMap<u8, String>,
    name_to_gid: HashMap<String, GlyphId>,
    gid_to_name: Vec<Option<String>>,
    standard_font: Option<StandardFont>,
}

impl CffKind {
    fn new(dict: &Dict<'_>) -> Option<Self> {
        let descriptor = dict.get::<Dict<'_>>(FONT_DESC)?;
        let data = descriptor.get::<Stream<'_>>(FONT_FILE3)?;
        let font = CffFontBlob::new(Arc::new(data.decoded().ok()?.to_vec()))?;

        let (encoding, encodings) = read_encoding(dict);
        let (widths, missing_width) = read_widths(dict, &descriptor)?;
        let standard_font = select_standard_font(dict, &descriptor).map(|(f, _)| f);
        let mut gid_to_name = vec![None; font.num_glyphs() as usize];
        let name_to_gid: HashMap<String, GlyphId> = font
            .glyph_names()
            .into_iter()
            .inspect(|(gid, name)| gid_to_name[gid.to_u32() as usize] = Some(name.clone()))
            .map(|(gid, name)| (name, gid))
            .collect();

        Some(Self {
            font,
            encoding,
            widths,
            missing_width,
            encodings,
            name_to_gid,
            gid_to_name,
            standard_font,
        })
    }

    fn map_code(&self, code: u8) -> GlyphId {
        let get_glyph = |entry: &str| {
            self.name_to_gid
                .get(entry)
                .copied()
                .or_else(|| self.name_to_gid.get(normalized_glyph_name(entry)).copied())
        };

        if let Some(entry) = self.encodings.get(&code) {
            get_glyph(entry)
        } else {
            match self.encoding {
                Encoding::BuiltIn => self.font.glyph_index(code),
                _ => self.encoding.map_code(code).and_then(get_glyph),
            }
        }
        .unwrap_or(GlyphId::NOTDEF)
    }

    fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        self.font.outline_glyph(glyph)
    }

    fn code_to_ps_name(&self, code: u8) -> Option<&str> {
        if let Some(entry) = self.encodings.get(&code) {
            Some(entry.as_str())
        } else {
            match self.encoding {
                Encoding::BuiltIn => self
                    .font
                    .glyph_index(code)
                    .and_then(|gid| self.gid_to_name.get(gid.to_u32() as usize))
                    .and_then(|name| name.as_deref()),
                _ => self.encoding.map_code(code),
            }
        }
    }

    fn glyph_width(&self, code: u8) -> Option<f32> {
        match self.widths.get(code as usize).copied() {
            Some(Width::Value(w)) => Some(w),
            Some(Width::Missing) => Some(self.missing_width),
            _ => {
                // If font looks like a standard font, get the width from there.
                let sf = self.standard_font?;
                self.code_to_ps_name(code)
                    .and_then(|name| sf.get_width(name))
            }
        }
    }

    fn char_code_to_unicode(&self, code: u8) -> Option<char> {
        self.code_to_ps_name(code).and_then(glyph_name_to_unicode)
    }
}
