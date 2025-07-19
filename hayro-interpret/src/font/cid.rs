use crate::font::blob::{CffFontBlob, OpenTypeFontBlob};
use crate::{InterpreterWarning, WarningSinkFn};
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use kurbo::{BezPath, Vec2};
use log::warn;
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId};
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Type0Font {
    font_type: FontType,
    horizontal: bool,
    dw: f32,
    dw2: (f32, f32),
    widths: HashMap<u16, f32>,
    widths2: HashMap<u16, [f32; 3]>,
    cid_to_gid_map: CidToGIdMap,
}

impl Type0Font {
    pub(crate) fn new(dict: &Dict, warning_sink: &WarningSinkFn) -> Option<Self> {
        let encoding = dict.get::<Name>(ENCODING).or_else(|| {
            warn!("CID fonts with custom encoding are currently unsupported");
            warning_sink(InterpreterWarning::UnsupportedFont);

            None
        })?;

        let horizontal = encoding.deref() == IDENTITY_H;

        let descendant_font = dict.get::<Array>(DESCENDANT_FONTS)?.iter::<Dict>().next()?;
        let font_descriptor = descendant_font.get::<Dict>(FONT_DESC)?;
        let font_type = FontType::new(&font_descriptor)?;

        let default_width = descendant_font.get::<f32>(DW).unwrap_or(1000.0);
        let dw2 = descendant_font
            .get::<[f32; 2]>(DW2)
            .map(|v| (v[0], v[1]))
            .unwrap_or((880.0, -1000.0));

        let widths = descendant_font
            .get::<Array>(W)
            .and_then(|a| read_widths(&a))
            .unwrap_or_default();
        let widths2 = descendant_font
            .get::<Array>(W2)
            .and_then(|a| read_widths2(&a))
            .unwrap_or_default();
        let cid_to_gid_map = CidToGIdMap::new(&descendant_font).unwrap_or_default();

        Some(Self {
            horizontal,
            font_type,
            dw: default_width,
            dw2,
            widths,
            widths2,
            cid_to_gid_map,
        })
    }

    pub(crate) fn map_code(&self, code: u16) -> GlyphId {
        match &self.font_type {
            FontType::TrueType(_) => self.cid_to_gid_map.map(code),
            FontType::Cff(c) => {
                let table = c.table();

                if table.is_cid() {
                    table
                        .glyph_index_by_cid(code)
                        .map(|g| GlyphId::new(g.0 as u32))
                        .unwrap_or(GlyphId::NOTDEF)
                } else {
                    GlyphId::new(code as u32)
                }
            }
        }
    }

    pub(crate) fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        match &self.font_type {
            FontType::TrueType(t) => t.outline_glyph(glyph),
            FontType::Cff(c) => c.outline_glyph(glyph),
        }
    }

    pub(crate) fn code_advance(&self, code: u16) -> Vec2 {
        if self.horizontal {
            Vec2::new(self.horizontal_width(code) as f64, 0.0)
        } else if let Some([w, _, _]) = self.widths2.get(&code) {
            Vec2::new(0.0, *w as f64)
        } else {
            Vec2::new(0.0, self.dw2.1 as f64)
        }
    }

    fn horizontal_width(&self, code: u16) -> f32 {
        self.widths.get(&code).copied().unwrap_or(self.dw)
    }

    pub(crate) fn is_horizontal(&self) -> bool {
        self.horizontal
    }

    pub(crate) fn code_len(&self) -> usize {
        2
    }

    pub(crate) fn origin_displacement(&self, code: u16) -> Vec2 {
        if self.is_horizontal() {
            Vec2::default()
        } else if let Some([_, v1, v2]) = self.widths2.get(&code) {
            Vec2::new(-*v1 as f64, -*v2 as f64)
        } else {
            Vec2::new(
                -self.horizontal_width(code) as f64 / 2.0,
                -self.dw2.0 as f64,
            )
        }
    }
}

#[derive(Debug)]
enum FontType {
    /// Type2 CID font.
    TrueType(OpenTypeFontBlob),
    /// Type0 CID font, backed by CFF font program (either via CIDFontType0C or OpenType).
    Cff(CffFontBlob),
}

impl FontType {
    fn new(descriptor: &Dict) -> Option<Self> {
        // Apparently there are some PDFs that have the wrong subtype,
        // so we just brute-force trying to parse the correct type to give
        // some leeway.

        if let Some(stream) = descriptor.get::<Stream>(FONT_FILE2) {
            let decoded = stream.decoded().ok()?;
            let data = Arc::new(decoded.to_vec());

            return Some(Self::TrueType(OpenTypeFontBlob::new(data, 0)?));
        } else if let Some(stream) = descriptor.get::<Stream>(FONT_FILE3) {
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
    fn new(dict: &Dict) -> Option<Self> {
        if let Some(name) = dict.get::<Name>(CID_TO_GID_MAP) {
            if name.deref() == IDENTITY {
                Some(CidToGIdMap::Identity)
            } else {
                None
            }
        } else if let Some(stream) = dict.get::<Stream>(CID_TO_GID_MAP) {
            let decoded = stream.decoded().ok()?;
            let mut map = HashMap::new();

            for (cid, gid) in decoded.chunks_exact(2).enumerate() {
                let gid = u16::from_be_bytes([gid[0], gid[1]]);

                map.insert(cid as u16, GlyphId::new(gid as u32));
            }

            Some(CidToGIdMap::Mapped(map))
        } else {
            None
        }
    }

    fn map(&self, code: u16) -> GlyphId {
        match self {
            CidToGIdMap::Identity => GlyphId::new(code as u32),
            CidToGIdMap::Mapped(map) => map.get(&code).copied().unwrap_or(GlyphId::NOTDEF),
        }
    }
}

fn read_widths(arr: &Array) -> Option<HashMap<u16, f32>> {
    let mut map = HashMap::new();
    let mut iter = arr.flex_iter();

    loop {
        if let Some((mut first, range)) = iter.next::<(u16, Array)>() {
            for width in range.iter::<f32>() {
                map.insert(first, width);
                first = first.checked_add(1)?;
            }
        } else if let Some((first, second, width)) = iter.next::<(u16, u16, f32)>() {
            for i in first..=second {
                map.insert(i, width);
            }
        } else {
            break;
        }
    }

    Some(map)
}

fn read_widths2(arr: &Array) -> Option<HashMap<u16, [f32; 3]>> {
    let mut map = HashMap::new();
    let mut iter = arr.flex_iter();

    loop {
        if let Some((mut first, range)) = iter.next::<(u16, Array)>() {
            let mut iter = range.iter::<f32>();

            while let Some(w) = iter.next() {
                let v1 = iter.next()?;
                let v2 = iter.next()?;
                map.insert(first, [w, v1, v2]);
                first = first.checked_add(1)?;
            }
        } else if let Some((first, second, w, v1, v2)) = iter.next::<(u16, u16, f32, f32, f32)>() {
            for i in first..=second {
                map.insert(i, [w, v1, v2]);
            }
        } else {
            break;
        }
    }

    Some(map)
}
