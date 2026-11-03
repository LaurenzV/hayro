use crate::FontLoader;
use crate::GlyphId;
use crate::generated::{glyph_names, mac_os_roman, mac_roman, standard};
use crate::program::{CmapPlatform, CmapSubtable, FontProgram, ProgramKind};
use crate::standard_font::{StandardFont, StandardKind, select_standard_font};
use crate::{
    Encoding, FallbackFontQuery, FontFlags, glyph_name_to_unicode, read_to_unicode,
    strip_subset_prefix, unicode_from_name,
};
use hayro_cmap::{BfString, CMap};
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Object;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use rustc_hash::FxHashMap;
use std::ops::Deref;

#[derive(Debug)]
pub(crate) struct TrueTypeFont {
    kind: Kind,
    to_unicode: Option<CMap>,
}

impl TrueTypeFont {
    pub(crate) fn new(dict: &Dict<'_>, loader: &FontLoader<'_>) -> Option<Self> {
        let to_unicode = read_to_unicode(dict, loader.cmap_resolver);

        if let Some(embedded) = EmbeddedKind::new(dict, loader) {
            return Some(Self {
                kind: Kind::Embedded(embedded),
                to_unicode,
            });
        }

        let fallback = || {
            let fallback_query = FallbackFontQuery::new(dict);
            let standard_font = fallback_query.pick_standard_font();

            Some(Self {
                kind: Kind::Standard(StandardKind::new_with_standard(
                    dict,
                    standard_font,
                    true,
                    &loader.programs,
                )?),
                to_unicode: to_unicode.clone(),
            })
        };

        if let Some(standard) = StandardKind::new(dict, &loader.programs) {
            Some(Self {
                kind: Kind::Standard(standard),
                to_unicode,
            })
        } else {
            fallback()
        }
    }

    pub(crate) fn program(&self) -> FontProgram {
        match &self.kind {
            Kind::Embedded(e) => e.base_font.clone(),
            Kind::Standard(s) => s.program.clone(),
        }
    }

    pub(crate) fn is_standard(&self) -> bool {
        matches!(self.kind, Kind::Standard(_))
    }

    pub(crate) fn glyph_stretch(&self, glyph: GlyphId, code: u8) -> f64 {
        match &self.kind {
            Kind::Embedded(_) => 1.0,
            Kind::Standard(s) => s.glyph_stretch(glyph, code),
        }
    }

    pub(crate) fn font_data(&self) -> Option<crate::FontData> {
        match &self.kind {
            Kind::Embedded(e) => Some(e.base_font.data().clone()),
            Kind::Standard(_) => None,
        }
    }

    pub(crate) fn postscript_name(&self) -> Option<&str> {
        match &self.kind {
            Kind::Embedded(e) => e.postscript_name.as_deref(),
            Kind::Standard(_) => None,
        }
    }

    pub(crate) fn weight(&self) -> Option<u32> {
        match &self.kind {
            Kind::Embedded(e) => e.base_font.weight(),
            Kind::Standard(s) => Some(if s.is_bold() { 700 } else { 400 }),
        }
    }

    pub(crate) fn is_italic(&self) -> bool {
        match &self.kind {
            Kind::Embedded(e) => {
                // Check PDF font flags first
                if let Some(flags) = &e.font_flags
                    && flags.contains(FontFlags::ITALIC)
                {
                    return true;
                }
                e.base_font.is_italic()
            }
            Kind::Standard(s) => s.is_italic(),
        }
    }

    pub(crate) fn is_serif(&self) -> bool {
        match &self.kind {
            Kind::Embedded(e) => e
                .font_flags
                .as_ref()
                .is_some_and(|f| f.contains(FontFlags::SERIF)),
            Kind::Standard(s) => s.is_serif(),
        }
    }

    pub(crate) fn is_monospace(&self) -> bool {
        match &self.kind {
            Kind::Embedded(e) => {
                // Check PDF font flags first
                if let Some(flags) = &e.font_flags
                    && flags.contains(FontFlags::FIXED_PITCH)
                {
                    return true;
                }
                e.base_font.is_monospace()
            }
            Kind::Standard(s) => s.is_monospace(),
        }
    }

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
        match &self.kind {
            Kind::Embedded(e) => e.map_code(code),
            Kind::Standard(s) => s.map_code(code),
        }
    }

    pub(crate) fn glyph_width(&self, code: u8) -> f32 {
        match &self.kind {
            Kind::Embedded(e) => e.glyph_width(code),
            Kind::Standard(s) => s.glyph_width(code).unwrap_or(0.0),
        }
    }

    pub(crate) fn char_code_to_unicode(&self, code: u32) -> Option<BfString> {
        if let Some(to_unicode) = &self.to_unicode
            && let Some(c) = to_unicode.lookup_bf_string(code)
        {
            return Some(c);
        }

        match &self.kind {
            Kind::Embedded(e) => e
                .code_to_name(code as u8)
                .and_then(|name| glyph_name_to_unicode(name, e.is_zapf_dingbats)),
            Kind::Standard(s) => s.char_code_to_unicode(code as u8),
        }

        // TODO: The test PDFs below fail (but mutool can render them correctly).
        // There is likely some other strategy that requires processing the font tables
        // tests/pdfs/custom/font_truetype_7.pdf
        // tests/pdfs/custom/font_truetype_6.pdf
    }
}

#[derive(Debug)]
enum Kind {
    Embedded(EmbeddedKind),
    Standard(StandardKind),
}

#[derive(Debug)]
struct EmbeddedKind {
    base_font: FontProgram,
    widths: Vec<Width>,
    missing_width: f32,
    font_flags: Option<FontFlags>,
    encoding: Encoding,
    differences: FxHashMap<u8, String>,
    code_to_glyph: Box<[GlyphId; 256]>,
    /// PostScript name from the PDF.
    postscript_name: Option<String>,
    is_zapf_dingbats: bool,
}

impl EmbeddedKind {
    fn new(dict: &Dict<'_>, loader: &FontLoader<'_>) -> Option<Self> {
        let descriptor = dict.get::<Dict<'_>>(FONT_DESC).unwrap_or_default();

        let font_flags = descriptor.get::<u32>(FLAGS).and_then(FontFlags::from_bits);

        let (widths, missing_width) = read_widths(dict, &descriptor)?;
        let (encoding, differences) = read_encoding(dict);
        let base_font = descriptor
            .get::<Stream<'_>>(FONT_FILE2)
            .and_then(|stream| loader.programs.load(&stream))
            .filter(|program| program.kind() == ProgramKind::Sfnt)?;

        let postscript_name = dict
            .get::<Name<'_>>(BASE_FONT)
            .map(|n| strip_subset_prefix(n.as_str()).to_string());
        let is_zapf_dingbats = matches!(
            select_standard_font(dict, &descriptor),
            Some((StandardFont::ZapfDingBats, _))
        );

        let mut kind = Self {
            base_font,
            differences,
            widths,
            missing_width,
            font_flags,
            encoding,
            code_to_glyph: Box::new([GlyphId::NOTDEF; 256]),
            postscript_name,
            is_zapf_dingbats,
        };
        let base_font = kind.base_font.clone();
        let subtables = base_font.cmap_subtables();
        kind.code_to_glyph = Box::new(std::array::from_fn(|code| {
            kind.lookup_glyph(code as u8, &subtables)
        }));

        Some(kind)
    }

    fn is_non_symbolic(&self) -> bool {
        self.font_flags
            .as_ref()
            .map(|f| f.contains(FontFlags::NON_SYMBOLIC))
            .unwrap_or(false)
    }

    fn code_to_name(&self, code: u8) -> Option<&str> {
        self.differences
            .get(&code)
            .map(|s| s.as_str())
            .or_else(|| self.encoding.map_code(code))
            // See PDFJS-6410 - PDF has no base encoding, so let's fallback to
            // standard here.
            .or_else(|| standard::get(code))
    }

    fn map_code(&self, code: u8) -> GlyphId {
        self.code_to_glyph[code as usize]
    }

    fn lookup_glyph(&self, code: u8, subtables: &[CmapSubtable<'_>]) -> GlyphId {
        // Some PDFs mistakenly embed an OpenType font with CFF outlines.
        if self.base_font.has_cff_outlines() {
            return self
                .code_to_name(code)
                .and_then(|name| self.base_font.glyph_by_name(name))
                .unwrap_or(GlyphId::NOTDEF);
        }

        let mut glyph = None;

        if self.is_non_symbolic() {
            let Some(lookup) = self.code_to_name(code) else {
                return GlyphId::NOTDEF;
            };

            for subtable in subtables {
                if subtable.platform() == CmapPlatform::Windows && subtable.encoding_id() == 1 {
                    glyph = glyph.or_else(|| {
                        glyph_names::get(lookup)
                            .map(|n| n.to_string())
                            .or_else(|| unicode_from_name(lookup).map(|n| n.to_string()))
                            .and_then(|n| n.chars().next())
                            .and_then(|c| subtable.map(u32::from(c)))
                            .filter(|g| *g != GlyphId::NOTDEF)
                    });
                }
            }

            for subtable in subtables {
                if subtable.platform() == CmapPlatform::Macintosh && subtable.encoding_id() == 0 {
                    glyph = glyph.or_else(|| {
                        mac_os_roman::get_inverse(lookup)
                            .or_else(|| mac_roman::get_inverse(lookup))
                            .and_then(|c| subtable.map(u32::from(c)))
                            .filter(|g| *g != GlyphId::NOTDEF)
                    });
                }
            }

            if glyph.is_none() {
                if let Some(gid) = self.base_font.glyph_by_name(lookup) {
                    glyph = Some(gid);
                } else if let Some(gid) = glyph_num_string(lookup) {
                    glyph = Some(GlyphId::new(gid));
                }
            }
        } else {
            for subtable in subtables {
                if subtable.platform() == CmapPlatform::Windows
                    && matches!(subtable.encoding_id(), 0 | 1)
                {
                    for offset in [0x0000_u32, 0xF000, 0xF100, 0xF200] {
                        glyph = glyph
                            .or_else(|| subtable.map(u32::from(code) + offset))
                            .filter(|g| *g != GlyphId::NOTDEF);
                    }
                } else if matches!(
                    subtable.platform(),
                    CmapPlatform::Macintosh | CmapPlatform::Unicode
                ) && subtable.encoding_id() == 0
                {
                    glyph = glyph
                        .or_else(|| subtable.map(u32::from(code)))
                        .filter(|g| *g != GlyphId::NOTDEF);
                }
            }
        }

        glyph.unwrap_or(GlyphId::NOTDEF)
    }

    fn glyph_width(&self, code: u8) -> f32 {
        match self.widths.get(code as usize).copied() {
            Some(Width::Value(w)) => w,
            Some(Width::Missing) => self.missing_width,
            _ => self
                .base_font
                .advance_width(self.map_code(code))
                .unwrap_or(0.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Width {
    Value(f32),
    Missing,
}

pub(crate) fn read_widths(dict: &Dict<'_>, descriptor: &Dict<'_>) -> Option<(Vec<Width>, f32)> {
    let mut widths = Vec::new();

    let first_char = dict.get::<usize>(FIRST_CHAR);
    let last_char = dict.get::<usize>(LAST_CHAR);
    let widths_arr = dict.get::<Array<'_>>(WIDTHS);
    let missing_width = descriptor.get::<f32>(MISSING_WIDTH).unwrap_or(0.0);

    if let (Some(fc), Some(lc), Some(w)) = (first_char, last_char, widths_arr) {
        let iter = w.iter::<f32>().take(lc.checked_sub(fc)?.checked_add(1)?);

        for _ in 0..fc {
            widths.push(Width::Missing);
        }

        for w in iter {
            widths.push(Width::Value(w));
        }

        while widths.len() <= (u8::MAX as usize) + 1 {
            widths.push(Width::Missing);
        }
    }

    Some((widths, missing_width))
}

pub(crate) fn read_encoding(dict: &Dict<'_>) -> (Encoding, FxHashMap<u8, String>) {
    fn get_encoding_base(dict: &Dict<'_>, name: Name<'_>) -> Encoding {
        match dict.get::<Name<'_>>(name.as_ref()) {
            Some(n) => match n.deref() {
                WIN_ANSI_ENCODING => Encoding::WinAnsi,
                MAC_ROMAN_ENCODING => Encoding::MacRoman,
                MAC_EXPERT_ENCODING => Encoding::MacExpert,
                STANDARD_ENCODING => Encoding::Standard,
                _ => Encoding::BuiltIn,
            },
            None => Encoding::BuiltIn,
        }
    }

    let mut map = FxHashMap::default();

    if let Some(encoding_dict) = dict.get::<Dict<'_>>(ENCODING) {
        if let Some(differences) = encoding_dict.get::<Array<'_>>(DIFFERENCES) {
            let entries = differences.iter::<Object<'_>>();

            let mut code = 0_i64;

            for obj in entries {
                match obj {
                    Object::Number(num) => {
                        code = num.as_i64();
                    }
                    Object::Name(name) => {
                        if let Ok(code) = code.try_into() {
                            map.insert(code, name.as_str().to_string());
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }

        (
            get_encoding_base(&encoding_dict, Name::new_unescaped(BASE_ENCODING)),
            map,
        )
    } else {
        (
            get_encoding_base(dict, Name::new_unescaped(ENCODING)),
            FxHashMap::default(),
        )
    }
}

fn glyph_num_string(s: &str) -> Option<u32> {
    if !s.starts_with('g') || s.len() < 2 {
        return None;
    }

    s[1..].parse::<u32>().ok()
}
