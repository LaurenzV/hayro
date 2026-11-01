/*!
Parsing the fonts of PDF files.

This crate turns PDF font dictionaries, together with their embedded font programs,
into fonts that can map character codes to glyphs and draw the outlines of those glyphs.

## Safety
This crate forbids unsafe code via a crate-level attribute.

## Cargo features
- `embed-fonts`: Embed a light-weight set of fonts (around 240KB) that can be used as a
  substitute for the 14 PDF standard fonts.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use crate::cid::Type0Font;
use crate::generated::{
    glyph_names, mac_expert, mac_os_roman, mac_roman, standard, tex_glyph_names, win_ansi,
    zapf_dings,
};
use crate::true_type::TrueTypeFont;
use crate::type1::Type1Font;
use crate::util::{CacheKey, hash128};
use bitflags::bitflags;
use hayro_syntax::object::Name;
use hayro_syntax::object::dict::keys::SUBTYPE;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::{Dict, Stream};
use kurbo::{Affine, BezPath, Vec2};
use outline::OutlineFont;
use skrifa::GlyphId;
use std::borrow::Cow;
use std::fmt::Debug;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

mod blob;
mod cid;
mod generated;
mod glyph_simulator;
mod outline;
mod standard_font;
mod true_type;
mod type1;
mod type3;
mod util;

pub use type3::Type3Font;

/// A callback function for resolving font queries.
///
/// The first argument is the raw data, the second argument is the index in case the font
/// is a TTC, otherwise it should be 0.
pub type FontResolverFn = Arc<dyn Fn(&FontQuery) -> Option<(FontData, u32)> + Send + Sync>;
/// A callback function for resolving cmap names to their files.
pub type CMapResolverFn = Arc<dyn Fn(CMapName<'_>) -> Option<&'static [u8]> + Send + Sync>;

/// The units per em that all outlines and metrics are scaled to.
pub const UNITS_PER_EM: f32 = 1000.0;

pub(crate) fn stretch_glyph(path: BezPath, expected_width: f32, actual_width: f32) -> BezPath {
    if actual_width != 0.0 && actual_width != expected_width {
        let stretch_factor = expected_width / actual_width;
        Affine::scale_non_uniform(stretch_factor as f64, 1.0) * path
    } else {
        path
    }
}

/// A container for the bytes of a PDF file.
pub type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// Strip the 6-character subset prefix from a PostScript font name.
///
/// PDF subset fonts use names like "ABCDEF+TimesNewRoman". This function
/// returns `TimesNewRoman` from such a name, or the original name if no
/// valid prefix is found.
pub(crate) fn strip_subset_prefix(name: &str) -> &str {
    match name.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 => rest,
        _ => name,
    }
}

use hayro_cmap::{BfString, CMap, CMapName, CharacterCollection};
pub use outline::OutlineFontData;
pub use standard_font::StandardFont;

/// An identifier that uniquely identifies a glyph, for caching purposes.
#[derive(Clone, Debug)]
pub struct GlyphIdentifier {
    id: GlyphId,
    font: OutlineFont,
}

impl GlyphIdentifier {
    /// Return the cache key of the glyph.
    pub fn cache_key(&self) -> u128 {
        hash128(&(self.id, self.font.cache_key()))
    }
}

/// A glyph defined by an outline.
#[derive(Clone, Debug)]
pub struct OutlineGlyph {
    pub(crate) id: GlyphId,
    pub(crate) font: OutlineFont,
    pub(crate) char_code: u32,
}

impl OutlineGlyph {
    /// Return the outline of the glyph, assuming an upem value of 1000.
    pub fn outline(&self) -> BezPath {
        self.font.outline_glyph(self.id, self.char_code)
    }

    /// Return the identifier of the glyph. You can use this to calculate the cache key
    /// for the glyph.
    ///
    /// Note that the `glyph_transform` attribute is not considered in the cache key of
    /// the identifier, only the glyph ID and the font.
    pub fn identifier(&self) -> GlyphIdentifier {
        GlyphIdentifier {
            id: self.id,
            font: self.font.clone(),
        }
    }

    /// Returns the Unicode code point for this glyph, if available.
    pub fn as_unicode(&self) -> Option<BfString> {
        self.font.char_code_to_unicode(self.char_code)
    }

    /// Get raw font bytes and metadata for downstream use.
    ///
    /// Returns `None` for Type1 fonts.
    pub fn font_data(&self) -> Option<OutlineFontData> {
        self.font.font_data()
    }

    /// Get the glyph ID within the font.
    pub fn glyph_id(&self) -> GlyphId {
        self.id
    }

    /// Get the original character code that selected this glyph.
    pub fn char_code(&self) -> u32 {
        self.char_code
    }

    /// Get the advance width for this glyph.
    ///
    /// The advance width is how far to move horizontally after drawing
    /// this glyph before drawing the next one.
    pub fn advance_width(&self) -> Option<f32> {
        self.font.glyph_advance_width(self.char_code)
    }

    /// Get the cache key for this glyph's font.
    ///
    /// This identifies the font uniquely, even when `font_data()` returns `None`
    /// (e.g., for Type1 fonts). Useful for grouping glyphs by font.
    pub fn font_cache_key(&self) -> u128 {
        self.font.cache_key()
    }
}

/// A PDF font.
#[derive(Clone, Debug)]
pub struct Font<'a>(u128, FontType<'a>);

impl<'a> Font<'a> {
    /// Load the font described by a font dictionary.
    pub fn new(
        dict: &Dict<'a>,
        font_resolver: &FontResolverFn,
        cmap_resolver: &CMapResolverFn,
    ) -> Option<Self> {
        let f_type = match dict.get::<Name<'_>>(SUBTYPE)?.deref() {
            TYPE1 | MM_TYPE1 => {
                FontType::Type1(Rc::new(Type1Font::new(dict, font_resolver, cmap_resolver)?))
            }
            // PDFBOX-5463: PDF viewers seem to accept OpenType as well.
            TRUE_TYPE | OPEN_TYPE => FontType::TrueType(Rc::new(TrueTypeFont::new(
                dict,
                font_resolver,
                cmap_resolver,
            )?)),
            TYPE0 => FontType::Type0(Rc::new(Type0Font::new(dict, font_resolver, cmap_resolver)?)),
            TYPE3 => FontType::Type3(Rc::new(Type3Font::new(dict, cmap_resolver)?)),
            _ => return None,
        };

        let cache_key = dict.cache_key();

        Some(Self(cache_key, f_type))
    }

    /// Load one of the 14 standard fonts.
    pub fn new_standard(
        standard_font: StandardFont,
        font_resolver: &FontResolverFn,
    ) -> Option<Self> {
        let font = Type1Font::new_standard(standard_font, font_resolver)?;

        Some(Self(0, FontType::Type1(Rc::new(font))))
    }

    /// Map a character code to a glyph.
    pub fn map_code(&self, code: u32) -> GlyphId {
        match &self.1 {
            FontType::Type1(f) => {
                debug_assert!(code <= u8::MAX as u32);

                f.map_code(code as u8)
            }
            FontType::TrueType(t) => {
                debug_assert!(code <= u8::MAX as u32);

                t.map_code(code as u8)
            }
            FontType::Type0(t) => t.map_code(code),
            FontType::Type3(t) => {
                debug_assert!(code <= u8::MAX as u32);

                t.map_code(code as u8)
            }
        }
    }

    /// Return the outline glyph that a glyph ID refers to, or `None` for Type 3 fonts.
    pub fn outline_glyph(&self, glyph: GlyphId, char_code: u32) -> Option<OutlineGlyph> {
        let font = match &self.1 {
            FontType::Type1(t) => OutlineFont::Type1(t.clone()),
            FontType::TrueType(t) => OutlineFont::TrueType(t.clone()),
            FontType::Type0(t) => OutlineFont::Type0(t.clone()),
            FontType::Type3(_) => return None,
        };

        Some(OutlineGlyph {
            id: glyph,
            font,
            char_code,
        })
    }

    /// Return the Type 3 font, if this is one.
    pub fn type3(&self) -> Option<&Rc<Type3Font<'a>>> {
        match &self.1 {
            FontType::Type3(t) => Some(t),
            _ => None,
        }
    }

    /// Return the advance of a character code.
    pub fn code_advance(&self, code: u32) -> Vec2 {
        match &self.1 {
            FontType::Type1(t) => {
                debug_assert!(code <= u8::MAX as u32);

                Vec2::new(t.glyph_width(code as u8).unwrap_or(0.0) as f64, 0.0)
            }
            FontType::TrueType(t) => {
                debug_assert!(code <= u8::MAX as u32);

                Vec2::new(t.glyph_width(code as u8) as f64, 0.0)
            }
            FontType::Type0(t) => t.code_advance(code),
            FontType::Type3(t) => {
                debug_assert!(code <= u8::MAX as u32);

                Vec2::new(t.glyph_width(code as u8) as f64, 0.0)
            }
        }
    }

    /// Return the displacement of the glyph origin of a character code.
    pub fn origin_displacement(&self, code: u32) -> Vec2 {
        match &self.1 {
            FontType::Type1(_) => Vec2::default(),
            FontType::TrueType(_) => Vec2::default(),
            FontType::Type0(t) => t.origin_displacement(code),
            FontType::Type3(_) => Vec2::default(),
        }
    }

    /// Read the next character code from a string, returning the code and its length.
    pub fn read_code(&self, bytes: &[u8], offset: usize) -> (u32, usize) {
        match &self.1 {
            FontType::Type1(_) => (bytes[offset] as u32, 1),
            FontType::TrueType(_) => (bytes[offset] as u32, 1),
            FontType::Type0(t) => t.read_code(bytes, offset),
            FontType::Type3(_) => (bytes[offset] as u32, 1),
        }
    }

    /// Whether the font is written horizontally.
    pub fn is_horizontal(&self) -> bool {
        match &self.1 {
            FontType::Type1(_) => true,
            FontType::TrueType(_) => true,
            FontType::Type0(t) => t.is_horizontal(),
            FontType::Type3(_) => true,
        }
    }
}

impl Font<'_> {
    /// Return the cache key of the font.
    pub fn cache_key(&self) -> u128 {
        self.0
    }
}

#[derive(Clone, Debug)]
enum FontType<'a> {
    Type1(Rc<Type1Font>),
    TrueType(Rc<TrueTypeFont>),
    Type0(Rc<Type0Font>),
    Type3(Rc<Type3Font<'a>>),
}

#[derive(Debug)]
enum Encoding {
    Standard,
    MacRoman,
    WinAnsi,
    MacExpert,
    BuiltIn,
}

impl Encoding {
    fn map_code(&self, code: u8) -> Option<&'static str> {
        if code == 0 {
            return Some(".notdef");
        }
        match self {
            Self::Standard => standard::get(code),
            Self::MacRoman => mac_roman::get(code).or_else(|| mac_os_roman::get(code)),
            Self::WinAnsi => win_ansi::get(code),
            Self::MacExpert => mac_expert::get(code),
            Self::BuiltIn => None,
        }
    }
}

/// The font stretch.
#[derive(Debug, Copy, Clone)]
pub enum FontStretch {
    /// Normal.
    Normal,
    /// Ultra condensed.
    UltraCondensed,
    /// Extra condensed.
    ExtraCondensed,
    /// Condensed.
    Condensed,
    /// Semi condensed.
    SemiCondensed,
    /// Semi expanded.
    SemiExpanded,
    /// Expanded.
    Expanded,
    /// Extra expanded.
    ExtraExpanded,
    /// Ultra expanded.
    UltraExpanded,
}

impl FontStretch {
    fn from_string(s: &str) -> Self {
        match s {
            "UltraCondensed" => Self::UltraCondensed,
            "ExtraCondensed" => Self::ExtraCondensed,
            "Condensed" => Self::Condensed,
            "SemiCondensed" => Self::SemiCondensed,
            "SemiExpanded" => Self::SemiExpanded,
            "Expanded" => Self::Expanded,
            "ExtraExpanded" => Self::ExtraExpanded,
            "UltraExpanded" => Self::UltraExpanded,
            _ => Self::Normal,
        }
    }
}

bitflags! {
    /// Bitflags describing various characteristics of fonts.
    #[derive(Debug)]
    pub(crate) struct FontFlags: u32 {
        const FIXED_PITCH = 1 << 0;
        const SERIF = 1 << 1;
        const SYMBOLIC = 1 << 2;
        const SCRIPT = 1 << 3;
        const NON_SYMBOLIC = 1 << 5;
        const ITALIC = 1 << 6;
        const ALL_CAP = 1 << 16;
        const SMALL_CAP = 1 << 17;
        const FORCE_BOLD = 1 << 18;
    }
}

/// A query for a font.
pub enum FontQuery {
    /// A query for one of the 14 PDF standard fonts.
    Standard(StandardFont),
    /// A query for a font that is not embedded in the PDF file.
    ///
    /// Note that this type of query is currently not supported,
    /// but will be implemented in the future.
    Fallback(FallbackFontQuery),
}

/// A query for a font with specific properties.
#[derive(Debug, Clone)]
pub struct FallbackFontQuery {
    /// The postscript name of the font.
    pub post_script_name: Option<String>,
    /// The name of the font.
    pub font_name: Option<String>,
    /// The family of the font.
    pub font_family: Option<String>,
    /// The stretch of the font.
    pub font_stretch: FontStretch,
    /// The weight of the font.
    pub font_weight: u32,
    /// Whether the font is monospaced.
    pub is_fixed_pitch: bool,
    /// Whether the font is serif.
    pub is_serif: bool,
    /// Whether the font is italic.
    pub is_italic: bool,
    /// Whether the font is bold.
    pub is_bold: bool,
    /// Whether the font is small cap.
    pub is_small_cap: bool,
    /// The character collection (registry/ordering) if this is a CID font.
    pub character_collection: Option<CharacterCollection>,
}

impl FallbackFontQuery {
    pub(crate) fn new(dict: &Dict<'_>) -> Self {
        let post_script_name = dict
            .get::<Name<'_>>(BASE_FONT)
            .map(|n| strip_subset_prefix(n.as_str()).to_string());

        let mut data = Self {
            post_script_name,
            ..Default::default()
        };

        if let Some(descriptor) = dict.get::<Dict<'_>>(FONT_DESC) {
            data.font_name = dict
                .get::<Name<'_>>(FONT_NAME)
                .map(|n| strip_subset_prefix(n.as_str()).to_string());
            data.font_family = descriptor
                .get::<Name<'_>>(FONT_FAMILY)
                .map(|n| n.as_str().to_string());
            data.font_stretch = descriptor
                .get::<Name<'_>>(FONT_STRETCH)
                .map(|n| FontStretch::from_string(n.as_str()))
                .unwrap_or(FontStretch::Normal);
            data.font_weight = descriptor.get::<u32>(FONT_WEIGHT).unwrap_or(400);

            if let Some(flags) = descriptor
                .get::<u32>(FLAGS)
                .map(FontFlags::from_bits_truncate)
            {
                data.is_serif = flags.contains(FontFlags::SERIF);
                data.is_italic = flags.contains(FontFlags::ITALIC);
                data.is_small_cap = flags.contains(FontFlags::SMALL_CAP);
            }
        }

        data.is_italic |= data
            .post_script_name
            .as_ref()
            .is_some_and(|s| s.contains("Italic"));

        data.is_bold |= data
            .post_script_name
            .as_ref()
            .is_some_and(|s| s.contains("Bold"));

        data
    }

    /// Do a best-effort fallback to the 14 standard fonts based on the query.
    pub fn pick_standard_font(&self) -> StandardFont {
        if self.is_fixed_pitch {
            match (self.is_bold, self.is_italic) {
                (true, true) => StandardFont::CourierBoldOblique,
                (true, false) => StandardFont::CourierBold,
                (false, true) => StandardFont::CourierOblique,
                (false, false) => StandardFont::Courier,
            }
        } else if !self.is_serif {
            match (self.is_bold, self.is_italic) {
                (true, true) => StandardFont::HelveticaBoldOblique,
                (true, false) => StandardFont::HelveticaBold,
                (false, true) => StandardFont::HelveticaOblique,
                (false, false) => StandardFont::Helvetica,
            }
        } else {
            match (self.is_bold, self.is_italic) {
                (true, true) => StandardFont::TimesBoldItalic,
                (true, false) => StandardFont::TimesBold,
                (false, true) => StandardFont::TimesItalic,
                (false, false) => StandardFont::TimesRoman,
            }
        }
    }
}

impl Default for FallbackFontQuery {
    fn default() -> Self {
        Self {
            post_script_name: None,
            font_name: None,
            font_family: None,
            font_stretch: FontStretch::Normal,
            font_weight: 400,
            is_fixed_pitch: false,
            is_serif: false,
            is_italic: false,
            is_bold: false,
            is_small_cap: false,
            character_collection: None,
        }
    }
}

/// Convert a glyph name to its Unicode value, if possible, as described in the Adobe Glyph List
/// Specification (<https://github.com/adobe-type-tools/agl-specification>), with the AGL
/// extended by glyph names used in the Computer Modern and AMS fonts.
pub(crate) fn glyph_name_to_unicode(name: &str, is_zapf_dingbats: bool) -> Option<BfString> {
    let base = name.split('.').next().unwrap_or_default();
    let mut out = String::new();

    for component in base.split('_') {
        if let Some(c) = is_zapf_dingbats
            .then(|| zapf_dings::glyph_name_to_unicode(component))
            .flatten()
        {
            out.push(c);
        } else if let Some(unicode_str) =
            glyph_names::get(component).or_else(|| tex_glyph_names::get(component))
        {
            out.push_str(unicode_str);
        } else if let Some(unicode_str) = uni_component(component) {
            out.push_str(&unicode_str);
        } else if let Some(c) = u_component(component) {
            out.push(c);
        }
    }

    bf_string_from_str(&out)
}

fn uni_component(component: &str) -> Option<String> {
    let digits = component.strip_prefix("uni")?.as_bytes();

    if digits.is_empty() || digits.len() % 4 != 0 {
        return None;
    }

    digits
        .as_chunks::<4>()
        .0
        .iter()
        .map(|group| char::from_u32(parse_uppercase_hex(group)?))
        .collect()
}

fn u_component(component: &str) -> Option<char> {
    let digits = component.strip_prefix('u')?.as_bytes();

    if !(4..=6).contains(&digits.len()) {
        return None;
    }

    char::from_u32(parse_uppercase_hex(digits)?)
}

fn parse_uppercase_hex(digits: &[u8]) -> Option<u32> {
    digits.iter().try_fold(0_u32, |value, digit| {
        let digit = match digit {
            b'0'..=b'9' => digit - b'0',
            b'A'..=b'F' => digit - b'A' + 10,
            _ => return None,
        };

        Some(value * 16 + u32::from(digit))
    })
}

fn bf_string_from_str(s: &str) -> Option<BfString> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(BfString::Char(c)),
        (Some(_), Some(_)) => Some(BfString::String(s.to_string())),
        (None, _) => None,
    }
}

pub(crate) fn unicode_from_name(name: &str) -> Option<char> {
    let convert = |input: &str| u32::from_str_radix(input, 16).ok().and_then(char::from_u32);

    name.starts_with("uni")
        .then(|| name.get(3..).and_then(convert))
        .or_else(|| {
            name.starts_with("u")
                .then(|| name.get(1..).and_then(convert))
        })
        .flatten()
}

pub(crate) fn read_to_unicode(dict: &Dict<'_>, cmap_resolver: &CMapResolverFn) -> Option<CMap> {
    dict.get::<Stream<'_>>(TO_UNICODE)
        .and_then(|s| s.decoded_data().ok())
        // See PDFJS-11915, where `Identity-H` is used for `ToUnicode`. I don't
        // believe it's valid, but at least mupdf seems to be able to deal with it.
        .or_else(|| {
            dict.get::<Name<'_>>(TO_UNICODE)
                .and_then(|name| (cmap_resolver)(CMapName::from_bytes(name.as_ref())))
                .map(|d| Cow::Owned(d.to_vec()))
        })
        .and_then(|data| {
            let cmap_resolver = cmap_resolver.clone();
            CMap::parse(&data, move |name| (cmap_resolver)(name))
        })
}

// When mapping to glyphs, some fonts might only have a glyph for the "normalized"
// name.
pub(crate) fn normalized_glyph_name(mut name: &str) -> &str {
    if name == "nbspace" {
        name = "space";
    }

    if name == "sfthyphen" {
        name = "hyphen";
    }

    name
}

#[cfg(test)]
mod tests {
    use super::glyph_name_to_unicode;
    use hayro_cmap::BfString;

    fn map(name: &str) -> Option<String> {
        glyph_name_to_unicode(name, false).map(|unicode| match unicode {
            BfString::Char(c) => c.to_string(),
            BfString::String(s) => s,
        })
    }

    #[test]
    fn agl_specification_examples() {
        assert_eq!(map("Lcommaaccent").as_deref(), Some("\u{013B}"));
        assert_eq!(map("uni20AC0308").as_deref(), Some("\u{20AC}\u{0308}"));
        assert_eq!(map("u1040C").as_deref(), Some("\u{1040C}"));
        assert_eq!(map("uniD801DC0C"), None);
        assert_eq!(map("uni20ac"), None);
        assert_eq!(
            map("Lcommaaccent_uni20AC0308_u1040C.alternate").as_deref(),
            Some("\u{013B}\u{20AC}\u{0308}\u{1040C}")
        );
        assert_eq!(map("foo"), None);
        assert_eq!(map(".notdef"), None);
    }

    #[test]
    fn components() {
        assert_eq!(map("dalethatafpatah").as_deref(), Some("\u{05D3}\u{05B2}"));
        assert_eq!(map("hyphen.case").as_deref(), Some("-"));
        assert_eq!(map("f_f_l").as_deref(), Some("ffl"));
        assert_eq!(map("f_g102").as_deref(), Some("f"));
        assert_eq!(map("uni000A0020").as_deref(), Some("\n "));
        assert_eq!(map("u41"), None);
        assert_eq!(map("u110000"), None);
    }
}
