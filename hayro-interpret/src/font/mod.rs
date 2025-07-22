//! Interacting with the different kinds of PDF fonts.

use crate::cache::Cache;
use crate::context::Context;
use crate::device::Device;
use crate::font::cid::Type0Font;
use crate::font::generated::{mac_expert, mac_os_roman, mac_roman, standard, win_ansi};
use crate::font::true_type::TrueTypeFont;
use crate::font::type1::Type1Font;
use crate::font::type3::Type3;
use crate::interpret::state::State;
use crate::{CacheKey, FontResolverFn, InterpreterSettings, Paint, WarningSinkFn};
use bitflags::bitflags;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::dict::keys::SUBTYPE;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::page::Resources;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, Vec2};
use log::warn;
use outline::OutlineFont;
use skrifa::GlyphId;
use std::fmt::Debug;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

mod blob;
mod cid;
mod generated;
mod glyph_simulator;
pub(crate) mod outline;
mod standard_font;
mod true_type;
mod type1;
pub(crate) mod type3;

pub(crate) const UNITS_PER_EM: f32 = 1000.0;

/// A container for the bytes of a PDF file.
pub type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;

use crate::util::hash128;
pub use standard_font::StandardFont;

/// A glyph that can be drawn.
pub enum Glyph<'a> {
    /// A glyph defined by an outline.
    Outline(OutlineGlyph),
    /// A type3 glyph, defined by PDF drawing instructions.
    Type3(Box<Type3Glyph<'a>>),
}

impl Glyph<'_> {
    pub(crate) fn glyph_transform(&self) -> Affine {
        match self {
            Glyph::Outline(o) => o.glyph_transform,
            Glyph::Type3(s) => s.glyph_transform,
        }
    }
}

/// An identifier that uniquely identifies a glyph, for caching purposes.
#[derive(Clone, Debug)]
pub struct GlyphIdentifier {
    id: GlyphId,
    font: OutlineFont,
}

impl CacheKey for GlyphIdentifier {
    fn cache_key(&self) -> u128 {
        hash128(&(self.id, self.font.cache_key()))
    }
}

/// A glyph defined by an outline.
#[derive(Clone, Debug)]
pub struct OutlineGlyph {
    pub(crate) id: GlyphId,
    pub(crate) font: OutlineFont,
    /// A transform that should be applied to the glyph before drawing.
    ///
    /// Note that this transform will not automatically be applied when calling `outline`.
    pub glyph_transform: Affine,
}

impl OutlineGlyph {
    /// Return the outline of the glyph, assuming an upem value of 1000.
    pub fn outline(&self) -> BezPath {
        self.font.outline_glyph(self.id)
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
}

/// A type3 glyph.
pub struct Type3Glyph<'a> {
    pub(crate) font: Rc<Type3<'a>>,
    pub(crate) glyph_id: GlyphId,
    pub(crate) state: State<'a>,
    pub(crate) parent_resources: Resources<'a>,
    pub(crate) cache: Cache,
    pub(crate) glyph_transform: Affine,
    pub(crate) xref: &'a XRef,
    pub(crate) settings: InterpreterSettings,
}

/// A glyph defined by PDF drawing instructions.
impl<'a> Type3Glyph<'a> {
    /// Draw the type3 glyph to the given device.
    pub fn interpret(&self, device: &mut impl Device, paint: &Paint) {
        self.font.render_glyph(self, paint, device);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Font<'a>(u128, FontType<'a>);

impl<'a> Font<'a> {
    pub(crate) fn new(
        dict: &Dict<'a>,
        resolver: &FontResolverFn,
        warning_sink: &WarningSinkFn,
    ) -> Option<Self> {
        let f_type = match dict.get::<Name>(SUBTYPE)?.deref() {
            TYPE1 | MM_TYPE1 => FontType::Type1(Rc::new(Type1Font::new(dict, resolver)?)),
            TRUE_TYPE => TrueTypeFont::new(dict)
                .map(Rc::new)
                .map(FontType::TrueType)
                .or_else(|| {
                    Type1Font::new(dict, resolver)
                        .map(Rc::new)
                        .map(FontType::Type1)
                })?,
            TYPE0 => FontType::Type0(Rc::new(Type0Font::new(dict, warning_sink)?)),
            TYPE3 => FontType::Type3(Rc::new(Type3::new(dict))),
            f => {
                warn!(
                    "unimplemented font type {:?}",
                    std::str::from_utf8(f).unwrap_or("unknown type")
                );

                return None;
            }
        };

        let cache_key = dict.cache_key();

        Some(Self(cache_key, f_type))
    }

    pub(crate) fn map_code(&self, code: u16) -> GlyphId {
        match &self.1 {
            FontType::Type1(f) => {
                debug_assert!(code <= u8::MAX as u16);

                f.map_code(code as u8)
            }
            FontType::TrueType(t) => {
                debug_assert!(code <= u8::MAX as u16);

                t.map_code(code as u8)
            }
            FontType::Type0(t) => t.map_code(code),
            FontType::Type3(t) => {
                debug_assert!(code <= u8::MAX as u16);

                t.map_code(code as u8)
            }
        }
    }

    pub(crate) fn get_glyph(
        &self,
        glyph: GlyphId,
        ctx: &mut Context<'a>,
        resources: &Resources<'a>,
        origin_displacement: Vec2,
    ) -> Glyph<'a> {
        let glyph_transform = ctx.get().text_state.full_transform()
            * Affine::scale(1.0 / UNITS_PER_EM as f64)
            * Affine::translate(origin_displacement);

        match &self.1 {
            FontType::Type1(t) => {
                let font = OutlineFont::Type1(t.clone());
                Glyph::Outline(OutlineGlyph {
                    id: glyph,
                    font,
                    glyph_transform,
                })
            }
            FontType::TrueType(t) => {
                let font = OutlineFont::TrueType(t.clone());
                Glyph::Outline(OutlineGlyph {
                    id: glyph,
                    font,
                    glyph_transform,
                })
            }
            FontType::Type0(t) => {
                let font = OutlineFont::Type0(t.clone());
                Glyph::Outline(OutlineGlyph {
                    id: glyph,
                    font,
                    glyph_transform,
                })
            }
            FontType::Type3(t) => {
                let shape_glyph = Type3Glyph {
                    font: t.clone(),
                    glyph_id: glyph,
                    state: ctx.get().clone(),
                    parent_resources: resources.clone(),
                    cache: ctx.object_cache.clone(),
                    xref: ctx.xref,
                    settings: ctx.settings.clone(),
                    glyph_transform,
                };

                Glyph::Type3(Box::new(shape_glyph))
            }
        }
    }

    pub(crate) fn code_advance(&self, code: u16) -> Vec2 {
        match &self.1 {
            FontType::Type1(t) => {
                debug_assert!(code <= u8::MAX as u16);

                Vec2::new(t.glyph_width(code as u8).unwrap_or(0.0) as f64, 0.0)
            }
            FontType::TrueType(t) => {
                debug_assert!(code <= u8::MAX as u16);

                Vec2::new(t.glyph_width(code as u8) as f64, 0.0)
            }
            FontType::Type0(t) => t.code_advance(code),
            FontType::Type3(t) => {
                debug_assert!(code <= u8::MAX as u16);

                Vec2::new(t.glyph_width(code as u8) as f64, 0.0)
            }
        }
    }

    pub(crate) fn origin_displacement(&self, code: u16) -> Vec2 {
        match &self.1 {
            FontType::Type1(_) => Vec2::default(),
            FontType::TrueType(_) => Vec2::default(),
            FontType::Type0(t) => t.origin_displacement(code),
            FontType::Type3(_) => Vec2::default(),
        }
    }

    pub(crate) fn code_len(&self) -> usize {
        match &self.1 {
            FontType::Type1(_) => 1,
            FontType::TrueType(_) => 1,
            FontType::Type0(t) => t.code_len(),
            FontType::Type3(_) => 1,
        }
    }

    pub(crate) fn is_horizontal(&self) -> bool {
        match &self.1 {
            FontType::Type1(_) => true,
            FontType::TrueType(_) => true,
            FontType::Type0(t) => t.is_horizontal(),
            FontType::Type3(_) => true,
        }
    }
}

impl CacheKey for Font<'_> {
    fn cache_key(&self) -> u128 {
        self.0
    }
}

#[derive(Clone, Debug)]
enum FontType<'a> {
    Type1(Rc<Type1Font>),
    TrueType(Rc<TrueTypeFont>),
    Type0(Rc<Type0Font>),
    Type3(Rc<Type3<'a>>),
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
            Encoding::Standard => standard::get(code),
            Encoding::MacRoman => mac_roman::get(code).or_else(|| mac_os_roman::get(code)),
            Encoding::WinAnsi => win_ansi::get(code),
            Encoding::MacExpert => mac_expert::get(code),
            Encoding::BuiltIn => None,
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
            "UltraCondensed" => FontStretch::UltraCondensed,
            "ExtraCondensed" => FontStretch::ExtraCondensed,
            "Condensed" => FontStretch::Condensed,
            "SemiCondensed" => FontStretch::SemiCondensed,
            "SemiExpanded" => FontStretch::SemiExpanded,
            "Expanded" => FontStretch::Expanded,
            "ExtraExpanded" => FontStretch::ExtraExpanded,
            "UltraExpanded" => FontStretch::UltraExpanded,
            _ => FontStretch::Normal,
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
}

impl FallbackFontQuery {
    pub(crate) fn new(dict: &Dict) -> Self {
        let mut data = Self::default();

        let remove_subset_prefix = |s: String| {
            if s.contains("+") {
                s.chars().skip(7).collect()
            } else {
                s
            }
        };

        data.post_script_name = dict
            .get::<Name>(BASE_FONT)
            .map(|n| remove_subset_prefix(n.as_str().to_string()));

        if let Some(descriptor) = dict.get::<Dict>(FONT_DESC) {
            data.font_name = dict
                .get::<Name>(FONT_NAME)
                .map(|n| remove_subset_prefix(n.as_str().to_string()));
            data.font_family = descriptor
                .get::<Name>(FONT_FAMILY)
                .map(|n| n.as_str().to_string());
            data.font_stretch = descriptor
                .get::<Name>(FONT_STRETCH)
                .map(|n| FontStretch::from_string(n.as_str()))
                .unwrap_or(FontStretch::Normal);
            data.font_weight = descriptor.get::<u32>(FONT_WEIGHT).unwrap_or(400);

            if let Some(flags) = descriptor
                .get::<u32>(FLAGS)
                .map(FontFlags::from_bits_truncate)
            {
                data.is_serif = flags.contains(FontFlags::SERIF);
                data.is_italic = flags.contains(FontFlags::ITALIC)
                    || data
                        .post_script_name
                        .as_ref()
                        .is_some_and(|s| s.contains("Italic"));
                data.is_small_cap = flags.contains(FontFlags::SMALL_CAP);
                data.is_bold = data
                    .post_script_name
                    .as_ref()
                    .is_some_and(|s| s.contains("Bold"));
            }
        }

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
        }
    }
}
