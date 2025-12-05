use crate::font::blob::{CffFontBlob, Type1FontBlob};
use crate::font::cmap::CMap;
use crate::font::generated::glyph_names;
use crate::font::glyph_simulator::GlyphSimulator;
use crate::font::standard_font::{StandardFont, StandardFontBlob, select_standard_font};
use crate::font::true_type::{read_encoding, read_widths};
use crate::font::{Encoding, FallbackFontQuery, FontQuery, glyph_name_to_unicode, read_to_unicode};
use crate::{CacheKey, FontResolverFn};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{FONT_DESC, FONT_FILE, FONT_FILE3};
use kurbo::{Affine, BezPath};
use log::warn;
use skrifa::GlyphId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Type1Font(u128, Kind, Option<CMap>);

impl Type1Font {
    pub(crate) fn new(dict: &Dict<'_>, resolver: &FontResolverFn) -> Option<Self> {
        let cache_key = dict.cache_key();

        let to_unicode = read_to_unicode(dict);

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

        let inner = if let Some(standard) = StandardKind::new(dict, resolver) {
            Self(cache_key, Kind::Standard(standard), to_unicode)
        } else if is_cff(dict) {
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
        } else {
            return fallback();
        };

        Some(inner)
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

    pub(crate) fn char_code_to_unicode(&self, char_code: u32) -> Option<char> {
        if let Some(to_unicode) = &self.2
            && let Some(unicode) = to_unicode.lookup_code(char_code)
        {
            // Skip null character mappings and fall back to glyph name
            // lookup. Some PDFs have incorrect ToUnicode mappings that map
            // to U+0000.
            if unicode != 0 {
                return char::from_u32(unicode);
            }
        }

        let code = char_code as u8;
        match &self.1 {
            Kind::Standard(s) => s.char_code_to_unicode(code),
            Kind::Cff(c) => c.char_code_to_unicode(code),
            Kind::Type1(t) => t.char_code_to_unicode(code),
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

#[derive(Debug)]
struct StandardKind {
    base_font: StandardFont,
    base_font_blob: StandardFontBlob,
    encoding: Encoding,
    widths: Vec<f32>,
    fallback: bool,
    glyph_to_code: RefCell<HashMap<GlyphId, u8>>,
    encodings: HashMap<u8, String>,
}

impl StandardKind {
    fn new(dict: &Dict<'_>, resolver: &FontResolverFn) -> Option<Self> {
        Self::new_with_standard(dict, select_standard_font(dict)?, false, resolver)
    }

    fn new_with_standard(
        dict: &Dict<'_>,
        base_font: StandardFont,
        fallback: bool,
        resolver: &FontResolverFn,
    ) -> Option<Self> {
        let descriptor = dict.get::<Dict<'_>>(FONT_DESC).unwrap_or_default();
        let widths = read_widths(dict, &descriptor);

        let (encoding, encoding_map) = read_encoding(dict);
        let (blob, index) = resolver(&FontQuery::Standard(base_font))?;
        let base_font_blob = StandardFontBlob::from_data(blob, index)?;

        Some(Self {
            base_font,
            base_font_blob,
            widths,
            encodings: encoding_map,
            glyph_to_code: RefCell::new(HashMap::new()),
            fallback,
            encoding,
        })
    }

    fn code_to_ps_name(&self, code: u8) -> Option<&str> {
        let bf = self.base_font;

        self.encodings
            .get(&code)
            .map(String::as_str)
            .or_else(|| match self.encoding {
                Encoding::BuiltIn => bf.code_to_name(code),
                _ => self.encoding.map_code(code),
            })
    }

    fn map_code(&self, code: u8) -> GlyphId {
        let result = self
            .code_to_ps_name(code)
            .and_then(|c| {
                self.base_font_blob.name_to_glyph(c).or_else(|| {
                    // If the font doesn't have a POST table, try to map via unicode instead.
                    glyph_names::get(c).and_then(|c| {
                        self.base_font_blob
                            .unicode_to_glyph(c.chars().nth(0).unwrap() as u32)
                    })
                })
            })
            .unwrap_or(GlyphId::NOTDEF);
        self.glyph_to_code.borrow_mut().insert(result, code);

        result
    }

    fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        let path = self.base_font_blob.outline_glyph(glyph);

        // If the font was not embedded in the file and we are using a standard font as a substitute,
        // we stretch the glyph so it matches the width of the standard font.
        if self.fallback
            && let Some(code) = self.glyph_to_code.borrow().get(&glyph).copied()
            && let Some(should_width) = self.glyph_width(code)
            && let Some(actual_width) = self
                .code_to_ps_name(code)
                .and_then(|name| self.base_font.get_width(name))
            && actual_width != 0.0
        {
            let stretch_factor = should_width / actual_width;

            return Affine::scale_non_uniform(stretch_factor as f64, 1.0) * path;
        }

        path
    }

    fn glyph_width(&self, code: u8) -> Option<f32> {
        self.widths.get(code as usize).copied().or_else(|| {
            self.code_to_ps_name(code)
                .and_then(|c| self.base_font.get_width(c))
        })
    }

    fn char_code_to_unicode(&self, code: u8) -> Option<char> {
        self.code_to_ps_name(code).and_then(glyph_name_to_unicode)
    }
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
    widths: Vec<f32>,
    encodings: HashMap<u8, String>,
    glyph_simulator: GlyphSimulator,
}

impl Type1Kind {
    fn new(dict: &Dict<'_>) -> Option<Self> {
        let descriptor = dict.get::<Dict<'_>>(FONT_DESC)?;
        let data = descriptor.get::<Stream<'_>>(FONT_FILE)?;
        let font = Type1FontBlob::new(Arc::new(data.decoded().ok()?.to_vec()))?;

        let (encoding, encodings) = read_encoding(dict);
        let widths = read_widths(dict, &descriptor);

        let glyph_simulator = GlyphSimulator::new();

        Some(Self {
            font,
            encoding,
            glyph_simulator,
            widths,
            encodings,
        })
    }

    fn string_to_glyph(&self, string: &str) -> GlyphId {
        self.glyph_simulator.string_to_glyph(string)
    }

    fn map_code(&self, code: u8) -> GlyphId {
        let table = self.font.table();

        let get_glyph = |entry: &str| self.string_to_glyph(entry);

        if let Some(entry) = self.encodings.get(&code) {
            Some(get_glyph(entry))
        } else {
            match self.encoding {
                Encoding::BuiltIn => table.code_to_string(code).map(get_glyph),
                _ => self.encoding.map_code(code).map(get_glyph),
            }
        }
        .unwrap_or(GlyphId::NOTDEF)
    }

    fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        self.font.outline_glyph(
            self.glyph_simulator
                .glyph_to_string(glyph)
                .unwrap()
                .as_str(),
        )
    }

    fn glyph_width(&self, code: u8) -> Option<f32> {
        self.widths.get(code as usize).copied()
    }

    fn char_code_to_unicode(&self, code: u8) -> Option<char> {
        let glyph_name = if let Some(entry) = self.encodings.get(&code) {
            Some(entry.as_str())
        } else {
            match self.encoding {
                Encoding::BuiltIn => self.font.table().code_to_string(code),
                _ => self.encoding.map_code(code),
            }
        };

        glyph_name.and_then(glyph_name_to_unicode)
    }
}

#[derive(Debug)]
struct CffKind {
    font: CffFontBlob,
    encoding: Encoding,
    widths: Vec<f32>,
    encodings: HashMap<u8, String>,
}

impl CffKind {
    fn new(dict: &Dict<'_>) -> Option<Self> {
        let descriptor = dict.get::<Dict<'_>>(FONT_DESC)?;
        let data = descriptor.get::<Stream<'_>>(FONT_FILE3)?;
        let font = CffFontBlob::new(Arc::new(data.decoded().ok()?.to_vec()))?;

        let (encoding, encodings) = read_encoding(dict);
        let widths = read_widths(dict, &descriptor);

        Some(Self {
            font,
            encoding,
            widths,
            encodings,
        })
    }

    fn map_code(&self, code: u8) -> GlyphId {
        let table = self.font.table();

        let get_glyph = |entry: &str| {
            table
                .glyph_index_by_name(entry)
                .map(|g| GlyphId::new(g.0 as u32))
        };

        if let Some(entry) = self.encodings.get(&code) {
            get_glyph(entry)
        } else {
            match self.encoding {
                Encoding::BuiltIn => table.glyph_index(code).map(|g| GlyphId::new(g.0 as u32)),
                _ => self.encoding.map_code(code).and_then(get_glyph),
            }
        }
        .unwrap_or(GlyphId::NOTDEF)
    }

    fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        self.font.outline_glyph(glyph)
    }

    fn glyph_width(&self, code: u8) -> Option<f32> {
        self.widths.get(code as usize).copied()
    }

    fn char_code_to_unicode(&self, code: u8) -> Option<char> {
        let glyph_name = if let Some(entry) = self.encodings.get(&code) {
            Some(entry.as_str())
        } else {
            let table = self.font.table();
            match self.encoding {
                Encoding::BuiltIn => table.glyph_index(code).and_then(|g| table.glyph_name(g)),
                _ => self.encoding.map_code(code),
            }
        };

        glyph_name.and_then(glyph_name_to_unicode)
    }
}
