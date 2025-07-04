use crate::font::blob::{
    COURIER_BOLD, COURIER_BOLD_ITALIC, COURIER_ITALIC, COURIER_REGULAR, CffFontBlob,
    HELVETICA_BOLD, HELVETICA_BOLD_ITALIC, HELVETICA_ITALIC, HELVETICA_REGULAR, OpenTypeFontBlob,
    TIMES_BOLD, TIMES_ITALIC, TIMES_REGULAR, TIMES_ROMAN_BOLD_ITALIC, ZAPF_DINGS_BAT,
};
use crate::font::generated::{metrics, standard, symbol, zapf_dings};
use crate::font::{FontData, blob};
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::dict::keys::{BASE_FONT, P};
use hayro_syntax::object::name::Name;
use kurbo::BezPath;
use skrifa::GlyphId16;
use skrifa::raw::TableProvider;
use std::collections::HashMap;
use std::ops::Deref;

#[derive(Copy, Clone, Debug)]
pub(crate) enum StandardFont {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    ZapfDingBats,
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

    pub(crate) fn from_font_data(data: &FontData) -> Self {
        if data.is_fixed_pitch {
            match (data.is_bold, data.is_italic) {
                (true, true) => StandardFont::CourierBoldOblique,
                (true, false) => StandardFont::CourierBold,
                (false, true) => StandardFont::CourierOblique,
                (false, false) => StandardFont::Courier,
            }
        } else if !data.is_serif {
            match (data.is_bold, data.is_italic) {
                (true, true) => StandardFont::HelveticaBoldOblique,
                (true, false) => StandardFont::HelveticaBold,
                (false, true) => StandardFont::HelveticaOblique,
                (false, false) => StandardFont::Helvetica,
            }
        } else {
            match (data.is_bold, data.is_italic) {
                (true, true) => StandardFont::TimesBoldItalic,
                (true, false) => StandardFont::TimesBold,
                (false, true) => StandardFont::TimesItalic,
                (false, false) => StandardFont::TimesRoman,
            }
        }
    }

    pub(crate) fn get_blob(&self) -> StandardFontBlob {
        match self {
            StandardFont::Helvetica => StandardFontBlob::new_otf(HELVETICA_REGULAR.clone()),
            StandardFont::HelveticaBold => StandardFontBlob::new_otf(HELVETICA_BOLD.clone()),
            StandardFont::HelveticaOblique => StandardFontBlob::new_otf(HELVETICA_ITALIC.clone()),
            StandardFont::HelveticaBoldOblique => {
                StandardFontBlob::new_otf(HELVETICA_BOLD_ITALIC.clone())
            }
            StandardFont::Courier => StandardFontBlob::new_otf(COURIER_REGULAR.clone()),
            StandardFont::CourierBold => StandardFontBlob::new_otf(COURIER_BOLD.clone()),
            StandardFont::CourierOblique => StandardFontBlob::new_otf(COURIER_ITALIC.clone()),
            StandardFont::CourierBoldOblique => {
                StandardFontBlob::new_otf(COURIER_BOLD_ITALIC.clone())
            }
            StandardFont::TimesRoman => StandardFontBlob::new_otf(TIMES_REGULAR.clone()),
            StandardFont::TimesBold => StandardFontBlob::new_otf(TIMES_BOLD.clone()),
            StandardFont::TimesItalic => StandardFontBlob::new_otf(TIMES_ITALIC.clone()),
            StandardFont::TimesBoldItalic => {
                StandardFontBlob::new_otf(TIMES_ROMAN_BOLD_ITALIC.clone())
            }
            StandardFont::ZapfDingBats => StandardFontBlob::new_cff(ZAPF_DINGS_BAT.clone()),
            StandardFont::Symbol => StandardFontBlob::new_cff(blob::SYMBOL.clone()),
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
            name = "hyphen"
        }

        match self {
            StandardFont::Helvetica => metrics::HELVETICA.get(name).copied(),
            StandardFont::HelveticaBold => metrics::HELVETICA_BOLD.get(name).copied(),
            StandardFont::HelveticaOblique => metrics::HELVETICA_OBLIQUE.get(name).copied(),
            StandardFont::HelveticaBoldOblique => {
                metrics::HELVETICA_BOLD_OBLIQUE.get(name).copied()
            }
            StandardFont::Courier => metrics::COURIER.get(name).copied(),
            StandardFont::CourierBold => metrics::COURIER_BOLD.get(name).copied(),
            StandardFont::CourierOblique => metrics::COURIER_OBLIQUE.get(name).copied(),
            StandardFont::CourierBoldOblique => metrics::COURIER_BOLD_OBLIQUE.get(name).copied(),
            StandardFont::TimesRoman => metrics::TIMES_ROMAN.get(name).copied(),
            StandardFont::TimesBold => metrics::TIMES_BOLD.get(name).copied(),
            StandardFont::TimesItalic => metrics::TIMES_ITALIC.get(name).copied(),
            StandardFont::TimesBoldItalic => metrics::TIMES_BOLD_ITALIC.get(name).copied(),
            StandardFont::ZapfDingBats => metrics::ZAPF_DING_BATS.get(name).copied(),
            StandardFont::Symbol => metrics::SYMBOL.get(name).copied(),
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            StandardFont::Helvetica => "Helvetica",
            StandardFont::HelveticaBold => "Helvetica Bold",
            StandardFont::HelveticaOblique => "Helvetica Oblique",
            StandardFont::HelveticaBoldOblique => "Helvetica Bold Oblique",
            StandardFont::Courier => "Courier",
            StandardFont::CourierBold => "Courier Bold",
            StandardFont::CourierOblique => "Courier Oblique",
            StandardFont::CourierBoldOblique => "Courier Bold Oblique",
            StandardFont::TimesRoman => "Times Roman",
            StandardFont::TimesBold => "Times Bold",
            StandardFont::TimesItalic => "Times Italic",
            StandardFont::TimesBoldItalic => "Times Bold Italic",
            StandardFont::ZapfDingBats => "Zapf Dingbats",
            StandardFont::Symbol => "Symbol",
        }
    }
}

pub(crate) fn select_standard_font(dict: &Dict) -> Option<StandardFont> {
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

    pub(crate) fn outline_glyph(&self, glyph: skrifa::GlyphId) -> BezPath {
        // Standard fonts have empty outlines for these, but in Liberation Sans
        // they are a .notdef rectangle.
        if glyph == skrifa::GlyphId::NOTDEF {
            return BezPath::new();
        }

        match self {
            StandardFontBlob::Cff(blob) => blob.outline_glyph(glyph),
            StandardFontBlob::Otf(blob, _) => blob.outline_glyph(glyph),
        }
    }
}
