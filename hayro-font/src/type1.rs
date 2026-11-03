use crate::GlyphId;
use crate::program::FontProgram;
use crate::standard_font::{StandardFont, StandardKind, select_standard_font};
use crate::true_type::{Width, read_encoding, read_widths};
use crate::{
    Encoding, FallbackFontQuery, glyph_name_to_unicode, normalized_glyph_name, read_to_unicode,
};
use crate::{FontLoader, ProgramLoader};
use hayro_cmap::{BfString, CMap};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{FONT_DESC, FONT_FILE, FONT_FILE3};
use rustc_hash::FxHashMap;

#[derive(Debug)]
pub(crate) struct Type1Font(Kind, Option<CMap>);

impl Type1Font {
    pub(crate) fn new(dict: &Dict<'_>, loader: &FontLoader<'_>) -> Option<Self> {
        let to_unicode = read_to_unicode(dict, loader.cmap_resolver);

        let fallback = || {
            // TODO: Actually use fallback fonts
            let fallback_query = FallbackFontQuery::new(dict);
            let standard_font = fallback_query.pick_standard_font();

            Some(Self(
                Kind::Standard(StandardKind::new_with_standard(
                    dict,
                    standard_font,
                    true,
                    &loader.programs,
                )?),
                to_unicode.clone(),
            ))
        };

        let inner = if let Some(embedded) = EmbeddedKind::new(dict, loader) {
            Self(Kind::Embedded(embedded), to_unicode)
        } else if has_font_file(dict) {
            return fallback();
        } else if let Some(standard) = StandardKind::new(dict, &loader.programs) {
            Self(Kind::Standard(standard), to_unicode)
        } else {
            return fallback();
        };

        Some(inner)
    }

    pub(crate) fn new_standard(font: StandardFont, loader: &ProgramLoader<'_>) -> Option<Self> {
        let dict = Dict::default();
        let standard = StandardKind::new_with_standard(&dict, font, true, loader)?;

        Some(Self(Kind::Standard(standard), None))
    }

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
        match &self.0 {
            Kind::Standard(s) => s.map_code(code),
            Kind::Embedded(e) => e.map_code(code),
        }
    }

    pub(crate) fn program(&self) -> FontProgram {
        match &self.0 {
            Kind::Standard(s) => s.program.clone(),
            Kind::Embedded(e) => e.program.clone(),
        }
    }

    pub(crate) fn is_standard(&self) -> bool {
        matches!(self.0, Kind::Standard(_))
    }

    pub(crate) fn glyph_stretch(&self, glyph: GlyphId, code: u8) -> f64 {
        match &self.0 {
            Kind::Standard(s) => s.glyph_stretch(glyph, code),
            Kind::Embedded(_) => 1.0,
        }
    }

    pub(crate) fn glyph_width(&self, code: u8) -> Option<f32> {
        match &self.0 {
            Kind::Standard(s) => s.glyph_width(code),
            Kind::Embedded(e) => e.glyph_width(code),
        }
    }

    pub(crate) fn char_code_to_unicode(&self, char_code: u32) -> Option<BfString> {
        if let Some(to_unicode) = &self.1
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
        match &self.0 {
            Kind::Standard(s) => s.char_code_to_unicode(code),
            Kind::Embedded(e) => e.char_code_to_unicode(code),
        }
    }
}

#[derive(Debug)]
enum Kind {
    Standard(StandardKind),
    Embedded(EmbeddedKind),
}

#[derive(Debug)]
struct EmbeddedKind {
    program: FontProgram,
    encoding: Encoding,
    widths: Vec<Width>,
    missing_width: f32,
    encodings: FxHashMap<u8, String>,
    standard_font: Option<StandardFont>,
}

impl EmbeddedKind {
    fn new(dict: &Dict<'_>, loader: &FontLoader<'_>) -> Option<Self> {
        let descriptor = dict.get::<Dict<'_>>(FONT_DESC)?;
        let data = descriptor
            .get::<Stream<'_>>(FONT_FILE3)
            .or_else(|| descriptor.get::<Stream<'_>>(FONT_FILE))?;
        let program = loader.programs.load(&data)?;

        let (encoding, encodings) = read_encoding(dict);
        let (widths, missing_width) = read_widths(dict, &descriptor)?;
        let standard_font = select_standard_font(dict, &descriptor).map(|(f, _)| f);

        Some(Self {
            program,
            encoding,
            widths,
            missing_width,
            encodings,
            standard_font,
        })
    }

    fn glyph_by_name(&self, name: &str) -> Option<GlyphId> {
        self.program
            .glyph_by_name(name)
            .or_else(|| self.program.glyph_by_name(normalized_glyph_name(name)))
    }

    fn map_code(&self, code: u8) -> GlyphId {
        if let Some(entry) = self.encodings.get(&code) {
            self.glyph_by_name(entry)
        } else {
            match self.encoding {
                // For OpenType programs, this uses the encoding of the CFF table instead of
                // the "cmap" table described in 9.6.5.4 of the PDF specification, as other
                // viewers do.
                Encoding::BuiltIn => self.program.builtin_encoding_glyph(code),
                _ => self
                    .encoding
                    .map_code(code)
                    .and_then(|name| self.glyph_by_name(name)),
            }
        }
        .unwrap_or(GlyphId::NOTDEF)
    }

    fn code_to_ps_name(&self, code: u8) -> Option<&str> {
        if let Some(entry) = self.encodings.get(&code) {
            Some(entry.as_str())
        } else {
            match self.encoding {
                Encoding::BuiltIn => self.program.builtin_encoding_glyph_name(code),
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

    fn char_code_to_unicode(&self, code: u8) -> Option<BfString> {
        let is_zapf_dingbats = matches!(self.standard_font, Some(StandardFont::ZapfDingBats));
        self.code_to_ps_name(code)
            .and_then(|name| glyph_name_to_unicode(name, is_zapf_dingbats))
    }
}

fn has_font_file(dict: &Dict<'_>) -> bool {
    dict.get::<Dict<'_>>(FONT_DESC)
        .is_some_and(|dict| dict.contains_key(FONT_FILE) || dict.contains_key(FONT_FILE3))
}
