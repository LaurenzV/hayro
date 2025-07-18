use crate::font::blob::{CffFontBlob, Type1FontBlob};
use crate::font::glyph_simulator::GlyphSimulator;
use crate::font::standard_font::{StandardFont, StandardFontBlob, select_standard_font};
use crate::font::true_type::{read_encoding, read_widths};
use crate::font::{Encoding, FallbackFontQuery, FontQuery};
use crate::{FontResolverFn, InterpreterSettings};
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::dict::keys::{FONT_DESC, FONT_FILE, FONT_FILE3};
use hayro_syntax::object::stream::Stream;
use kurbo::{Affine, BezPath};
use log::warn;
use skrifa::GlyphId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Type1Font(Kind);

impl Type1Font {
    pub(crate) fn new(dict: &Dict, resolver: &FontResolverFn) -> Option<Self> {
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

            Some(Self(Kind::Standard(StandardKind::new_with_standard(
                dict,
                standard_font,
                true,
                resolver,
            )?)))
        };

        let inner = if let Some(standard) = StandardKind::new(dict, resolver) {
            Self(Kind::Standard(standard))
        } else if is_cff(dict) {
            Self(Kind::Cff(CffKind::new(dict)?))
        } else if is_type1(dict) {
            if let Some(f) = Type1Kind::new(dict) {
                Self(Kind::Type1(f))
            } else {
                return fallback();
            }
        } else {
            return fallback();
        };

        Some(inner)
    }

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
        match &self.0 {
            Kind::Standard(s) => s.map_code(code),
            Kind::Type1(s) => s.map_code(code),
            Kind::Cff(c) => c.map_code(code),
        }
    }

    pub(crate) fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        match &self.0 {
            Kind::Standard(s) => s.outline_glyph(glyph),
            Kind::Cff(c) => c.outline_glyph(glyph),
            Kind::Type1(t) => t.outline_glyph(glyph),
        }
    }

    pub(crate) fn glyph_width(&self, code: u8) -> Option<f32> {
        match &self.0 {
            Kind::Standard(s) => s.glyph_width(code),
            Kind::Cff(c) => c.glyph_width(code),
            Kind::Type1(t) => t.glyph_width(code),
        }
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
    fn new(dict: &Dict, resolver: &FontResolverFn) -> Option<StandardKind> {
        Some(Self::new_with_standard(
            dict,
            select_standard_font(dict)?,
            false,
            resolver,
        )?)
    }

    fn new_with_standard(
        dict: &Dict,
        base_font: StandardFont,
        fallback: bool,
        resolver: &FontResolverFn,
    ) -> Option<Self> {
        let descriptor = dict.get::<Dict>(FONT_DESC).unwrap_or_default();
        let widths = read_widths(dict, &descriptor);

        let (encoding, encoding_map) = read_encoding(dict);
        let base_font_blob =
            StandardFontBlob::from_data(resolver(&FontQuery::Standard(base_font))?)?;

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
            .and_then(|c| self.base_font_blob.name_to_glyph(c))
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
}

fn is_cff(dict: &Dict) -> bool {
    dict.get::<Dict>(FONT_DESC)
        .map(|dict| dict.contains_key(FONT_FILE3))
        .unwrap_or(false)
}

fn is_type1(dict: &Dict) -> bool {
    dict.get::<Dict>(FONT_DESC)
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
    fn new(dict: &Dict) -> Option<Self> {
        let descriptor = dict.get::<Dict>(FONT_DESC)?;
        let data = descriptor.get::<Stream>(FONT_FILE)?;
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
}

#[derive(Debug)]
struct CffKind {
    font: CffFontBlob,
    encoding: Encoding,
    widths: Vec<f32>,
    encodings: HashMap<u8, String>,
}

impl CffKind {
    fn new(dict: &Dict) -> Option<Self> {
        let descriptor = dict.get::<Dict>(FONT_DESC)?;
        let data = descriptor.get::<Stream>(FONT_FILE3)?;
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
}
