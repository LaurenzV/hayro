use crate::FontResolverFn;
use crate::font::blob::{CffFontBlob, OpenTypeFontBlob};
use crate::font::generated::{glyph_names, metrics, standard, symbol, zapf_dings};
use crate::font::true_type::{Width, read_encoding, read_widths};
use crate::font::{
    Encoding, FontData, FontQuery, glyph_name_to_unicode, normalized_glyph_name, stretch_glyph,
    strip_subset_prefix,
};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::dict::keys::{BASE_FONT, FONT_DESC};
use kurbo::BezPath;
use skrifa::raw::TableProvider;
use skrifa::{GlyphId, GlyphId16};
use std::cell::RefCell;
use std::collections::HashMap;

/// The 14 standard fonts of PDF.
#[derive(Copy, Clone, Debug)]
pub enum StandardFont {
    /// Helvetica.
    Helvetica,
    /// Helvetica Bold.
    HelveticaBold,
    /// Helvetica Oblique.
    HelveticaOblique,
    /// Helvetica Bold Oblique.
    HelveticaBoldOblique,
    /// Courier.
    Courier,
    /// Courier Bold.
    CourierBold,
    /// Courier Oblique.
    CourierOblique,
    /// Courier Bold Oblique.
    CourierBoldOblique,
    /// Times Roman.
    TimesRoman,
    /// Times Bold.
    TimesBold,
    /// Times Italic.
    TimesItalic,
    /// Times Bold Italic.
    TimesBoldItalic,
    /// Zapf Dingbats - a decorative symbol font.
    ZapfDingBats,
    /// Symbol - a mathematical symbol font.
    Symbol,
}

impl StandardFont {
    pub(crate) fn code_to_name(&self, code: u8) -> Option<&'static str> {
        match self {
            Self::Symbol => symbol::get(code),
            // Note that this font does not return postscript character names,
            // but instead has a custom encoding.
            Self::ZapfDingBats => zapf_dings::get(code),
            _ => standard::get(code),
        }
    }

    pub(crate) fn get_width(&self, mut name: &str) -> Option<f32> {
        // <https://github.com/apache/pdfbox/blob/129aafe26548c1ff935af9c55cb40a996186c35f/pdfbox/src/main/java/org/apache/pdfbox/pdmodel/font/PDSimpleFont.java#L340>
        if name == ".notdef" {
            return Some(250.0);
        }

        name = normalized_glyph_name(name);

        match self {
            Self::Helvetica => metrics::HELVETICA.get(name).copied(),
            Self::HelveticaBold => metrics::HELVETICA_BOLD.get(name).copied(),
            Self::HelveticaOblique => metrics::HELVETICA_OBLIQUE.get(name).copied(),
            Self::HelveticaBoldOblique => metrics::HELVETICA_BOLD_OBLIQUE.get(name).copied(),
            Self::Courier => metrics::COURIER.get(name).copied(),
            Self::CourierBold => metrics::COURIER_BOLD.get(name).copied(),
            Self::CourierOblique => metrics::COURIER_OBLIQUE.get(name).copied(),
            Self::CourierBoldOblique => metrics::COURIER_BOLD_OBLIQUE.get(name).copied(),
            Self::TimesRoman => metrics::TIMES_ROMAN.get(name).copied(),
            Self::TimesBold => metrics::TIMES_BOLD.get(name).copied(),
            Self::TimesItalic => metrics::TIMES_ITALIC.get(name).copied(),
            Self::TimesBoldItalic => metrics::TIMES_BOLD_ITALIC.get(name).copied(),
            Self::ZapfDingBats => metrics::ZAPF_DING_BATS.get(name).copied(),
            Self::Symbol => metrics::SYMBOL.get(name).copied(),
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Helvetica => "Helvetica",
            Self::HelveticaBold => "Helvetica Bold",
            Self::HelveticaOblique => "Helvetica Oblique",
            Self::HelveticaBoldOblique => "Helvetica Bold Oblique",
            Self::Courier => "Courier",
            Self::CourierBold => "Courier Bold",
            Self::CourierOblique => "Courier Oblique",
            Self::CourierBoldOblique => "Courier Bold Oblique",
            Self::TimesRoman => "Times Roman",
            Self::TimesBold => "Times Bold",
            Self::TimesItalic => "Times Italic",
            Self::TimesBoldItalic => "Times Bold Italic",
            Self::ZapfDingBats => "Zapf Dingbats",
            Self::Symbol => "Symbol",
        }
    }

    /// Return the postscrit name of the font.
    pub fn postscript_name(&self) -> &'static str {
        match self {
            Self::Helvetica => "Helvetica",
            Self::HelveticaBold => "Helvetica-Bold",
            Self::HelveticaOblique => "Helvetica-Oblique",
            Self::HelveticaBoldOblique => "Helvetica-BoldOblique",
            Self::Courier => "Courier",
            Self::CourierBold => "Courier-Bold",
            Self::CourierOblique => "Courier-Oblique",
            Self::CourierBoldOblique => "Courier-BoldOblique",
            Self::TimesRoman => "Times-Roman",
            Self::TimesBold => "Times-Bold",
            Self::TimesItalic => "Times-Italic",
            Self::TimesBoldItalic => "Times-BoldItalic",
            Self::ZapfDingBats => "ZapfDingbats",
            Self::Symbol => "Symbol",
        }
    }

    pub(crate) fn is_bold(&self) -> bool {
        matches!(
            self,
            Self::HelveticaBold
                | Self::HelveticaBoldOblique
                | Self::CourierBold
                | Self::CourierBoldOblique
                | Self::TimesBold
                | Self::TimesBoldItalic
        )
    }

    pub(crate) fn is_italic(&self) -> bool {
        matches!(
            self,
            Self::HelveticaOblique
                | Self::HelveticaBoldOblique
                | Self::CourierOblique
                | Self::CourierBoldOblique
                | Self::TimesItalic
                | Self::TimesBoldItalic
        )
    }

    pub(crate) fn is_serif(&self) -> bool {
        matches!(
            self,
            Self::TimesRoman | Self::TimesBold | Self::TimesItalic | Self::TimesBoldItalic
        )
    }

    pub(crate) fn is_monospace(&self) -> bool {
        matches!(
            self,
            Self::Courier | Self::CourierBold | Self::CourierOblique | Self::CourierBoldOblique
        )
    }

    /// Return suitable font data for the given standard font.
    ///
    /// Currently, this will return the corresponding Foxit font, which is a set of permissibly
    /// licensed fonts that is also very light-weight.
    ///
    /// You can use the result of this method in your implementation of [`FontResolverFn`].
    ///
    /// [`FontResolverFn`]: crate::FontResolverFn
    #[cfg(feature = "embed-fonts")]
    pub fn get_font_data(&self) -> (FontData, u32) {
        use std::sync::Arc;

        let data = match self {
            Self::Helvetica => &include_bytes!("../../assets/FoxitSans.pfb")[..],
            Self::HelveticaBold => &include_bytes!("../../assets/FoxitSansBold.pfb")[..],
            Self::HelveticaOblique => &include_bytes!("../../assets/FoxitSansItalic.pfb")[..],
            Self::HelveticaBoldOblique => {
                &include_bytes!("../../assets/FoxitSansBoldItalic.pfb")[..]
            }
            Self::Courier => &include_bytes!("../../assets/FoxitFixed.pfb")[..],
            Self::CourierBold => &include_bytes!("../../assets/FoxitFixedBold.pfb")[..],
            Self::CourierOblique => &include_bytes!("../../assets/FoxitFixedItalic.pfb")[..],
            Self::CourierBoldOblique => {
                &include_bytes!("../../assets/FoxitFixedBoldItalic.pfb")[..]
            }
            Self::TimesRoman => &include_bytes!("../../assets/FoxitSerif.pfb")[..],
            Self::TimesBold => &include_bytes!("../../assets/FoxitSerifBold.pfb")[..],
            Self::TimesItalic => &include_bytes!("../../assets/FoxitSerifItalic.pfb")[..],
            Self::TimesBoldItalic => &include_bytes!("../../assets/FoxitSerifBoldItalic.pfb")[..],
            Self::ZapfDingBats => &include_bytes!("../../assets/FoxitDingbats.pfb")[..],
            Self::Symbol => {
                include_bytes!("../../assets/FoxitSymbol.pfb")
            }
        };

        (Arc::new(data), 0)
    }
}

pub(crate) fn select_standard_font(dict: &Dict<'_>) -> Option<StandardFont> {
    // See <https://github.com/apache/pdfbox/blob/4438b8fdc67a3a9ebfb194595d0e81f88b708a37/pdfbox/src/main/java/org/apache/pdfbox/pdmodel/font/FontMapperImpl.java#L62-L102>
    match strip_subset_prefix(dict.get::<Name>(BASE_FONT)?.as_str()) {
        "Helvetica" | "ArialMT" | "Arial" | "LiberationSans" | "NimbusSanL-Regu" => {
            Some(StandardFont::Helvetica)
        }
        "Helvetica-Bold"
        | "Arial-BoldMT"
        | "Arial-Bold"
        | "Arial,Bold"
        | "LiberationSans-Bold"
        | "NimbusSanL-Bold" => Some(StandardFont::HelveticaBold),
        "Helvetica-Oblique"
        | "Arial-ItalicMT"
        | "Arial-ItalicMT,Italic"
        | "Arial-Italic"
        | "Arial,Italic"
        | "Helvetica-Italic"
        | "Helvetica,Italic"
        | "LiberationSans-Italic"
        | "NimbusSanL-ReguItal" => Some(StandardFont::HelveticaOblique),
        "Helvetica-BoldOblique"
        | "Arial-BoldItalicMT"
        | "Helvetica-BoldItalic"
        | "Helvetica,BoldItalic"
        | "LiberationSans-BoldItalic"
        | "NimbusSanL-BoldItal" => Some(StandardFont::HelveticaBoldOblique),
        "Courier" | "CourierNew" | "CourierNewPSMT" | "LiberationMono" | "NimbusMonL-Regu" => {
            Some(StandardFont::Courier)
        }
        "Courier-Bold"
        | "Courier,Bold"
        | "CourierNewPS-BoldMT"
        | "CourierNew-Bold"
        | "LiberationMono-Bold"
        | "NimbusMonL-Bold" => Some(StandardFont::CourierBold),
        "Courier-Oblique"
        | "CourierNewPS-ItalicMT"
        | "CourierNew-Italic"
        | "LiberationMono-Italic"
        | "NimbusMonL-ReguObli" => Some(StandardFont::CourierOblique),
        "Courier-BoldOblique"
        | "CourierNewPS-BoldItalicMT"
        | "CourierNew-BoldItalic"
        | "LiberationMono-BoldItalic"
        | "NimbusMonL-BoldObli" => Some(StandardFont::CourierBoldOblique),
        "Times-Roman" | "Times New Roman" | "TimesNewRomanPSMT" | "TimesNewRoman"
        | "TimesNewRomanPS" | "LiberationSerif" | "NimbusRomNo9L-Regu" => {
            Some(StandardFont::TimesRoman)
        }
        "Times-Bold"
        | "TimesNewRomanPS-BoldMT"
        | "TimesNewRomanPS-Bold"
        | "TimesNewRoman-Bold"
        | "TimesNewRoman,Bold"
        | "LiberationSerif-Bold"
        | "NimbusRomNo9L-Medi" => Some(StandardFont::TimesBold),
        "Times-Italic"
        | "TimesNewRomanPS-ItalicMT"
        | "TimesNewRomanPS-Italic"
        | "TimesNewRoman-Italic"
        | "TimesNewRoman,Italic"
        | "LiberationSerif-Italic"
        | "NimbusRomNo9L-ReguItal" => Some(StandardFont::TimesItalic),
        "Times-BoldItalic"
        | "TimesNewRomanPS-BoldItalicMT"
        | "TimesNewRomanPS-BoldItalic"
        | "TimesNewRoman-BoldItalic"
        | "TimesNewRoman,BoldItalic"
        | "LiberationSerif-BoldItalic"
        | "NimbusRomNo9L-MediItal" => Some(StandardFont::TimesBoldItalic),
        "Symbol" | "SymbolMT" | "StandardSymL" => Some(StandardFont::Symbol),
        "ZapfDingbats" | "ZapfDingbatsITCbyBT-Regular" | "ZapfDingbatsITC" | "Dingbats" => {
            Some(StandardFont::ZapfDingBats)
        }
        _ => None,
    }
}

#[derive(Debug)]
pub(crate) enum StandardFontBlob {
    Cff(CffFontBlob),
    Otf(OpenTypeFontBlob, HashMap<String, GlyphId>),
}

impl StandardFontBlob {
    pub(crate) fn from_data(data: FontData, index: u32) -> Option<Self> {
        if let Some(blob) = CffFontBlob::new(data.clone()) {
            Some(Self::new_cff(blob))
        } else {
            OpenTypeFontBlob::new(data, index).map(Self::new_otf)
        }
    }

    pub(crate) fn new_cff(blob: CffFontBlob) -> Self {
        Self::Cff(blob)
    }

    pub(crate) fn new_otf(blob: OpenTypeFontBlob) -> Self {
        let mut glyph_names = HashMap::new();

        if let Ok(post) = blob.font_ref().post() {
            for i in 0..blob.num_glyphs() {
                if let Some(str) = post.glyph_name(GlyphId16::new(i)) {
                    glyph_names.insert(str.to_string(), GlyphId::new(i as u32));
                }
            }
        }

        Self::Otf(blob, glyph_names)
    }
}

impl StandardFontBlob {
    pub(crate) fn name_to_glyph(&self, name: &str) -> Option<GlyphId> {
        match self {
            Self::Cff(blob) => blob
                .table()
                .glyph_index_by_name(name)
                .map(|g| GlyphId::new(g.0 as u32)),
            Self::Otf(_, glyph_names) => glyph_names.get(name).copied(),
        }
    }

    pub(crate) fn unicode_to_glyph(&self, code: u32) -> Option<GlyphId> {
        match self {
            Self::Cff(_) => None,
            Self::Otf(blob, _) => blob
                .font_ref()
                .cmap()
                .ok()
                .and_then(|c| c.map_codepoint(code)),
        }
    }

    pub(crate) fn advance_width(&self, glyph: GlyphId) -> Option<f32> {
        match self {
            Self::Cff(_) => None,
            Self::Otf(blob, _) => blob.glyph_metrics().advance_width(glyph),
        }
    }

    pub(crate) fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        // Standard fonts have empty outlines for these, but in Liberation Sans
        // they are a .notdef rectangle.
        if glyph == GlyphId::NOTDEF {
            return BezPath::new();
        }

        match self {
            Self::Cff(blob) => blob.outline_glyph(glyph),
            Self::Otf(blob, _) => blob.outline_glyph(glyph),
        }
    }
}

#[derive(Debug)]
pub(crate) struct StandardKind {
    base_font: StandardFont,
    base_font_blob: StandardFontBlob,
    encoding: Encoding,
    widths: Vec<Width>,
    missing_width: f32,
    fallback: bool,
    glyph_to_code: RefCell<HashMap<GlyphId, u8>>,
    encodings: HashMap<u8, String>,
}

impl StandardKind {
    pub(crate) fn new(dict: &Dict<'_>, resolver: &FontResolverFn) -> Option<Self> {
        Self::new_with_standard(dict, select_standard_font(dict)?, false, resolver)
    }

    pub(crate) fn new_with_standard(
        dict: &Dict<'_>,
        base_font: StandardFont,
        fallback: bool,
        resolver: &FontResolverFn,
    ) -> Option<Self> {
        let descriptor = dict.get::<Dict<'_>>(FONT_DESC).unwrap_or_default();
        let (widths, missing_width) = read_widths(dict, &descriptor)?;

        let (mut encoding, encoding_map) = read_encoding(dict);

        // See PDFJS-16464: Ignore encodings for non-embedded Type1 symbol fonts.
        if matches!(base_font, StandardFont::Symbol | StandardFont::ZapfDingBats) {
            encoding = Encoding::BuiltIn;
        }

        let (blob, index) = resolver(&FontQuery::Standard(base_font))?;
        let base_font_blob = StandardFontBlob::from_data(blob, index)?;

        Some(Self {
            base_font,
            base_font_blob,
            widths,
            missing_width,
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

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
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

    pub(crate) fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        let path = self.base_font_blob.outline_glyph(glyph);

        // If the font was not embedded in the file and we are using a standard font as a substitute,
        // we stretch the glyph so it matches the width of the standard font.
        if self.fallback
            && let Some(code) = self.glyph_to_code.borrow().get(&glyph).copied()
            // Only apply glyph stretching if the width was specified explicitly (and don't use
            // missing width), otherwise yields some weird results in some of the tests.
            && let Some(Width::Value(should_width)) = self.widths.get(code as usize).copied()
            && let Some(actual_width) = self
                .base_font_blob
                .advance_width(glyph)
                .or_else(|| {
                    self.code_to_ps_name(code)
                        .and_then(|name| self.base_font.get_width(name))
                })
        {
            return stretch_glyph(path, should_width, actual_width);
        }

        path
    }

    pub(crate) fn glyph_width(&self, code: u8) -> Option<f32> {
        match self.widths.get(code as usize).copied() {
            Some(Width::Value(w)) => Some(w),
            Some(Width::Missing) => Some(self.missing_width),
            None => self
                .code_to_ps_name(code)
                .and_then(|c| self.base_font.get_width(c)),
        }
    }

    pub(crate) fn char_code_to_unicode(&self, code: u8) -> Option<char> {
        self.code_to_ps_name(code).and_then(glyph_name_to_unicode)
    }

    pub(crate) fn is_italic(&self) -> bool {
        self.base_font.is_italic()
    }

    pub(crate) fn is_bold(&self) -> bool {
        self.base_font.is_bold()
    }

    pub(crate) fn is_serif(&self) -> bool {
        self.base_font.is_serif()
    }

    pub(crate) fn is_monospace(&self) -> bool {
        self.base_font.is_monospace()
    }
}
