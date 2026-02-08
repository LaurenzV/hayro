use crate::CacheKey;
use crate::font::blob::{CffFontBlob, OpenTypeFontBlob};
use crate::font::{FontFlags, read_to_unicode, strip_subset_prefix};
use hayro_cmap::{CMap, UnicodeString, WritingMode};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::{Array, Object};
use kurbo::{BezPath, Vec2};
use log::warn;
use skrifa::attribute::Style;
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Type0Font {
    font_type: FontType,
    horizontal: bool,
    cache_key: u128,
    dw: f32,
    dw2: (f32, f32),
    widths: HashMap<u32, f32>,
    encoding: CMap,
    to_unicode: Option<CMap>,
    widths2: HashMap<u32, [f32; 3]>,
    cid_to_gid_map: CidToGIdMap,
    /// PostScript name from the PDF.
    postscript_name: Option<String>,
    /// Font flags from the font descriptor.
    font_flags: Option<FontFlags>,
}

impl Type0Font {
    pub(crate) fn new(dict: &Dict<'_>) -> Option<Self> {
        let cmap = read_encoding(&dict.get::<Object<'_>>(ENCODING)?)?;

        let horizontal = cmap.metadata().writing_mode == WritingMode::Horizontal;

        let descendant_font = dict
            .get::<Array<'_>>(DESCENDANT_FONTS)?
            .iter::<Dict<'_>>()
            .next()?;
        let font_descriptor = descendant_font.get::<Dict<'_>>(FONT_DESC)?;
        let font_type = FontType::new(&font_descriptor)?;

        let default_width = descendant_font.get::<f32>(DW).unwrap_or(1000.0);
        let dw2 = descendant_font
            .get::<[f32; 2]>(DW2)
            .map(|v| (v[0], v[1]))
            .unwrap_or((880.0, -1000.0));

        let widths = descendant_font
            .get::<Array<'_>>(W)
            .and_then(|a| read_widths(&a))
            .unwrap_or_default();
        let widths2 = descendant_font
            .get::<Array<'_>>(W2)
            .and_then(|a| read_widths2(&a))
            .unwrap_or_default();
        let cid_to_gid_map = CidToGIdMap::new(&descendant_font).unwrap_or_default();
        let cache_key = dict.cache_key();

        let to_unicode = read_to_unicode(dict);

        let postscript_name = dict
            .get::<Name>(BASE_FONT)
            .map(|n| strip_subset_prefix(n.as_str()).to_string());

        // Extract font flags from descriptor
        let font_flags = font_descriptor
            .get::<u32>(FLAGS)
            .and_then(FontFlags::from_bits);

        Some(Self {
            cache_key,
            horizontal,
            encoding: cmap,
            to_unicode,
            font_type,
            dw: default_width,
            dw2,
            widths,
            widths2,
            cid_to_gid_map,
            postscript_name,
            font_flags,
        })
    }

    pub(crate) fn map_code(&self, code: u32) -> GlyphId {
        let Some(cid) = self.code_to_cid(code) else {
            return GlyphId::NOTDEF;
        };

        match &self.font_type {
            FontType::TrueType(_) => self.cid_to_gid_map.map(cid as u16),
            FontType::Cff(c) => {
                let table = c.table();

                if table.is_cid() {
                    table
                        .glyph_index_by_cid(cid as u16)
                        .map(|g| GlyphId::new(g.0 as u32))
                        .unwrap_or(GlyphId::NOTDEF)
                } else {
                    self.cid_to_gid_map.map(cid as u16)
                }
            }
        }
    }

    fn code_to_cid(&self, code: u32) -> Option<u32> {
        for byte_len in 1..=4_u8 {
            if let Some(cid) = self.encoding.lookup_cid_code(code, byte_len) {
                return Some(cid);
            }
        }

        None
    }

    pub(crate) fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        match &self.font_type {
            FontType::TrueType(t) => t.outline_glyph(glyph),
            FontType::Cff(c) => c.outline_glyph(glyph),
        }
    }

    pub(crate) fn font_data(&self) -> crate::font::FontData {
        match &self.font_type {
            FontType::TrueType(t) => t.font_data(),
            FontType::Cff(c) => c.font_data(),
        }
    }

    /// Get the PostScript name.
    pub(crate) fn postscript_name(&self) -> Option<&str> {
        self.postscript_name.as_deref()
    }

    /// Get the font weight (100-900, 400=normal, 700=bold).
    ///
    /// Returns `None` if weight cannot be determined (CFF fonts or invalid weight).
    pub(crate) fn weight(&self) -> Option<u32> {
        match &self.font_type {
            FontType::TrueType(t) => {
                let weight = t.font_ref().attributes().weight.value().round() as u32;
                if weight > 0 { Some(weight) } else { None }
            }
            FontType::Cff(_) => None,
        }
    }

    /// Check if font is italic based on font flags or font attributes.
    pub(crate) fn is_italic(&self) -> bool {
        if let Some(flags) = &self.font_flags
            && flags.contains(FontFlags::ITALIC)
        {
            return true;
        }
        match &self.font_type {
            FontType::TrueType(t) => t.font_ref().attributes().style != Style::Normal,
            FontType::Cff(_) => false,
        }
    }

    /// Check if font is serif based on font flags.
    pub(crate) fn is_serif(&self) -> bool {
        self.font_flags
            .as_ref()
            .is_some_and(|f| f.contains(FontFlags::SERIF))
    }

    /// Check if font is monospace based on font flags or font metrics.
    pub(crate) fn is_monospace(&self) -> bool {
        if let Some(flags) = &self.font_flags
            && flags.contains(FontFlags::FIXED_PITCH)
        {
            return true;
        }
        match &self.font_type {
            FontType::TrueType(t) => {
                t.font_ref()
                    .metrics(
                        skrifa::instance::Size::unscaled(),
                        skrifa::instance::LocationRef::default(),
                    )
                    .is_monospace
            }
            FontType::Cff(_) => false,
        }
    }

    pub(crate) fn code_advance(&self, code: u32) -> Vec2 {
        let cid = self.code_to_cid(code).unwrap_or(0);
        if self.horizontal {
            Vec2::new(self.horizontal_width(cid) as f64, 0.0)
        } else if let Some([w, _, _]) = self.widths2.get(&cid) {
            Vec2::new(0.0, *w as f64)
        } else {
            Vec2::new(0.0, self.dw2.1 as f64)
        }
    }

    fn horizontal_width(&self, cid: u32) -> f32 {
        self.widths.get(&cid).copied().unwrap_or(self.dw)
    }

    pub(crate) fn is_horizontal(&self) -> bool {
        self.horizontal
    }

    pub(crate) fn read_code(&self, bytes: &[u8], offset: usize) -> (u32, usize) {
        let mut code = 0_u32;
        let remaining = bytes.len() - offset;

        for n in 0..4.min(remaining) {
            code = (code << 8) | bytes[offset + n] as u32;

            if self.encoding.lookup_cid_code(code, (n + 1) as u8).is_some() {
                return (code, n + 1);
            }
        }

        (0, 1)
    }

    pub(crate) fn origin_displacement(&self, code: u32) -> Vec2 {
        let cid = self.code_to_cid(code).unwrap_or(0);

        if self.is_horizontal() {
            Vec2::default()
        } else if let Some([_, v1, v2]) = self.widths2.get(&cid) {
            Vec2::new(-*v1 as f64, -*v2 as f64)
        } else {
            Vec2::new(-self.horizontal_width(cid) as f64 / 2.0, -self.dw2.0 as f64)
        }
    }

    pub(crate) fn char_code_to_unicode(&self, code: u32) -> Option<UnicodeString> {
        if let Some(to_unicode) = &self.to_unicode {
            return to_unicode.lookup_unicode_code(code);
        }

        // TODO: Implement CID collection mappings (Adobe-Japan1, Adobe-GB1, etc.).

        None
    }
}

impl CacheKey for Type0Font {
    fn cache_key(&self) -> u128 {
        self.cache_key
    }
}

#[derive(Debug)]
enum FontType {
    /// Type2 CID font.
    TrueType(OpenTypeFontBlob),
    /// Type0 CID font, backed by CFF font program (either via `CIDFontType0C` or OpenType).
    Cff(CffFontBlob),
}

impl FontType {
    fn new(descriptor: &Dict<'_>) -> Option<Self> {
        // Apparently there are some PDFs that have the wrong subtype,
        // so we just brute-force trying to parse the correct type to give
        // some leeway.

        if let Some(stream) = descriptor.get::<Stream<'_>>(FONT_FILE2) {
            let decoded = stream.decoded().ok()?;
            let data = Arc::new(decoded.to_vec());

            return Some(Self::TrueType(OpenTypeFontBlob::new(data, 0)?));
        } else if let Some(stream) = descriptor.get::<Stream<'_>>(FONT_FILE3) {
            let decoded = stream.decoded().ok()?;

            return match stream.dict().get::<Name>(SUBTYPE)?.deref() {
                CID_FONT_TYPE0C => {
                    let data = Arc::new(decoded.to_vec());

                    Some(Self::Cff(CffFontBlob::new(data)?))
                }
                OPEN_TYPE => {
                    let font_ref = FontRef::new(decoded.as_ref()).ok()?;
                    let cff_data = Arc::new(font_ref.cff().ok()?.offset_data().as_ref().to_vec());

                    Some(Self::Cff(CffFontBlob::new(cff_data)?))
                }
                _ => {
                    warn!("unknown subtype for FontFile3");

                    None
                }
            };
        }

        warn!("CID font didn't have an embededd font file");

        None
    }
}

#[derive(Debug, Default)]
enum CidToGIdMap {
    #[default]
    Identity,
    Mapped(HashMap<u16, GlyphId>),
}

impl CidToGIdMap {
    fn new(dict: &Dict<'_>) -> Option<Self> {
        if let Some(name) = dict.get::<Name>(CID_TO_GID_MAP) {
            if name.deref() == IDENTITY {
                Some(Self::Identity)
            } else {
                None
            }
        } else if let Some(stream) = dict.get::<Stream<'_>>(CID_TO_GID_MAP) {
            let decoded = stream.decoded().ok()?;
            let mut map = HashMap::new();

            for (cid, gid) in decoded.chunks_exact(2).enumerate() {
                let gid = u16::from_be_bytes([gid[0], gid[1]]);

                map.insert(cid as u16, GlyphId::new(gid as u32));
            }

            Some(Self::Mapped(map))
        } else {
            None
        }
    }

    fn map(&self, code: u16) -> GlyphId {
        match self {
            Self::Identity => GlyphId::new(code as u32),
            Self::Mapped(map) => map.get(&code).copied().unwrap_or(GlyphId::NOTDEF),
        }
    }
}

fn read_widths(arr: &Array<'_>) -> Option<HashMap<u32, f32>> {
    let mut map = HashMap::new();
    let mut iter = arr.flex_iter();

    loop {
        if let Some((mut first, range)) = iter.next::<(u32, Array<'_>)>() {
            for width in range.iter::<f32>() {
                map.insert(first, width);
                first = first.checked_add(1)?;
            }
        } else if let Some((first, second, width)) = iter.next::<(u32, u32, f32)>() {
            for i in first..=second {
                map.insert(i, width);
            }
        } else {
            break;
        }
    }

    Some(map)
}

fn read_widths2(arr: &Array<'_>) -> Option<HashMap<u32, [f32; 3]>> {
    let mut map = HashMap::new();
    let mut iter = arr.flex_iter();

    loop {
        if let Some((mut first, range)) = iter.next::<(u32, Array<'_>)>() {
            let mut iter = range.iter::<f32>();

            while let Some(w) = iter.next() {
                let v1 = iter.next()?;
                let v2 = iter.next()?;
                map.insert(first, [w, v1, v2]);
                first = first.checked_add(1)?;
            }
        } else if let Some((first, second, w, v1, v2)) = iter.next::<(u32, u32, f32, f32, f32)>() {
            for i in first..=second {
                map.insert(i, [w, v1, v2]);
            }
        } else {
            break;
        }
    }

    Some(map)
}

fn read_encoding(object: &Object<'_>) -> Option<CMap> {
    match object {
        Object::Name(n) => match n.deref() {
            IDENTITY_H => Some(CMap::identity_h()),
            IDENTITY_V => Some(CMap::identity_v()),
            _ => {
                warn!("built-in encodings are not supported yet: {n:?}");

                None
            }
        },
        Object::Stream(s) => {
            let decoded = s.decoded().ok()?;
            // TODO: Support usecmap
            CMap::parse(&decoded, |_| None)
        }
        _ => None,
    }
}
