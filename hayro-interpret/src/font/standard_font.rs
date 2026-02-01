use crate::font::FontData;
use crate::font::blob::{CffFontBlob, OpenTypeFontBlob};
use crate::font::generated::{metrics, standard, symbol, zapf_dings};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::dict::keys::BASE_FONT;
use kurbo::BezPath;
use skrifa::GlyphId16;
use skrifa::raw::TableProvider;
use std::collections::HashMap;
use std::ops::Deref;

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

        if name == "nbspace" {
            name = "space";
        }

        if name == "sfthyphen" {
            name = "hyphen";
        }

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
    match dict.get::<Name>(BASE_FONT)?.deref() {
        b"Helvetica" | b"ArialMT" | b"Arial" | b"LiberationSans" | b"NimbusSanL-Regu" => {
            Some(StandardFont::Helvetica)
        }
        b"Helvetica-Bold"
        | b"Arial-BoldMT"
        | b"Arial-Bold"
        | b"Arial,Bold"
        | b"LiberationSans-Bold"
        | b"NimbusSanL-Bold" => Some(StandardFont::HelveticaBold),
        b"Helvetica-Oblique"
        | b"Arial-ItalicMT"
        | b"Arial-Italic"
        | b"Helvetica-Italic"
        | b"LiberationSans-Italic"
        | b"NimbusSanL-ReguItal" => Some(StandardFont::HelveticaOblique),
        b"Helvetica-BoldOblique"
        | b"Arial-BoldItalicMT"
        | b"Helvetica-BoldItalic"
        | b"LiberationSans-BoldItalic"
        | b"NimbusSanL-BoldItal" => Some(StandardFont::HelveticaBoldOblique),
        b"Courier" | b"CourierNew" | b"CourierNewPSMT" | b"LiberationMono" | b"NimbusMonL-Regu" => {
            Some(StandardFont::Courier)
        }
        b"Courier-Bold"
        | b"CourierNewPS-BoldMT"
        | b"CourierNew-Bold"
        | b"LiberationMono-Bold"
        | b"NimbusMonL-Bold" => Some(StandardFont::CourierBold),
        b"Courier-Oblique"
        | b"CourierNewPS-ItalicMT"
        | b"CourierNew-Italic"
        | b"LiberationMono-Italic"
        | b"NimbusMonL-ReguObli" => Some(StandardFont::CourierOblique),
        b"Courier-BoldOblique"
        | b"CourierNewPS-BoldItalicMT"
        | b"CourierNew-BoldItalic"
        | b"LiberationMono-BoldItalic"
        | b"NimbusMonL-BoldObli" => Some(StandardFont::CourierBoldOblique),
        b"Times-Roman"
        | b"Times New Roman"
        | b"TimesNewRomanPSMT"
        | b"TimesNewRoman"
        | b"TimesNewRomanPS"
        | b"LiberationSerif"
        | b"NimbusRomNo9L-Regu" => Some(StandardFont::TimesRoman),
        b"Times-Bold"
        | b"TimesNewRomanPS-BoldMT"
        | b"TimesNewRomanPS-Bold"
        | b"TimesNewRoman-Bold"
        | b"LiberationSerif-Bold"
        | b"NimbusRomNo9L-Medi" => Some(StandardFont::TimesBold),
        b"Times-Italic"
        | b"TimesNewRomanPS-ItalicMT"
        | b"TimesNewRomanPS-Italic"
        | b"TimesNewRoman-Italic"
        | b"LiberationSerif-Italic"
        | b"NimbusRomNo9L-ReguItal" => Some(StandardFont::TimesItalic),
        b"Times-BoldItalic"
        | b"TimesNewRomanPS-BoldItalicMT"
        | b"TimesNewRomanPS-BoldItalic"
        | b"TimesNewRoman-BoldItalic"
        | b"LiberationSerif-BoldItalic"
        | b"NimbusRomNo9L-MediItal" => Some(StandardFont::TimesBoldItalic),
        b"Symbol" | b"SymbolMT" | b"StandardSymL" => Some(StandardFont::Symbol),
        b"ZapfDingbats"
        | b"ZapfDingbatsITCbyBT-Regular"
        | b"ZapfDingbatsITC"
        | b"Dingbats"
        | b"MS-Gothic" => Some(StandardFont::ZapfDingBats),
        _ => None,
    }
}

#[derive(Debug)]
pub(crate) enum StandardFontBlob {
    Cff(CffFontBlob),
    Otf(OpenTypeFontBlob, HashMap<String, skrifa::GlyphId>),
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
                    glyph_names.insert(str.to_string(), skrifa::GlyphId::new(i as u32));
                }
            }
        }

        Self::Otf(blob, glyph_names)
    }
}

impl StandardFontBlob {
    pub(crate) fn name_to_glyph(&self, name: &str) -> Option<skrifa::GlyphId> {
        match self {
            Self::Cff(blob) => blob
                .table()
                .glyph_index_by_name(name)
                .map(|g| skrifa::GlyphId::new(g.0 as u32)),
            Self::Otf(_, glyph_names) => glyph_names.get(name).copied(),
        }
    }

    pub(crate) fn unicode_to_glyph(&self, code: u32) -> Option<skrifa::GlyphId> {
        match self {
            Self::Cff(_) => None,
            Self::Otf(blob, _) => blob
                .font_ref()
                .cmap()
                .ok()
                .and_then(|c| c.map_codepoint(code)),
        }
    }

    pub(crate) fn outline_glyph(&self, glyph: skrifa::GlyphId) -> BezPath {
        // Standard fonts have empty outlines for these, but in Liberation Sans
        // they are a .notdef rectangle.
        if glyph == skrifa::GlyphId::NOTDEF {
            return BezPath::new();
        }

        match self {
            Self::Cff(blob) => blob.outline_glyph(glyph),
            Self::Otf(blob, _) => blob.outline_glyph(glyph),
        }
    }
}
