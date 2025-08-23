use crate::CacheKey;
use crate::font::blob::{CffFontBlob, OpenTypeFontBlob};
use crate::font::generated::{glyph_names, mac_os_roman, mac_roman};
use crate::font::{Encoding, FontFlags};
use crate::util::{CodeMapExt, OptionLog};
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Object;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use kurbo::BezPath;
use log::warn;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::cmap::PlatformId;
use skrifa::{GlyphId, GlyphId16};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct TrueTypeFont {
    cache_key: u128,
    base_font: OpenTypeFontBlob,
    widths: Vec<f32>,
    font_flags: Option<FontFlags>,
    glyph_names: HashMap<String, GlyphId>,
    encoding: Encoding,
    // Only used for PDFs that mistakenly embed a
    // CFF font.
    cff_blob: Option<CffFontBlob>,
    differences: HashMap<u8, String>,
    cached_mappings: RefCell<HashMap<u8, GlyphId>>,
}

impl TrueTypeFont {
    pub(crate) fn new(dict: &Dict) -> Option<TrueTypeFont> {
        let descriptor = dict.get::<Dict>(FONT_DESC).unwrap_or_default();

        let font_flags = descriptor.get::<u32>(FLAGS).and_then(FontFlags::from_bits);

        let widths = read_widths(dict, &descriptor);
        let (encoding, differences) = read_encoding(dict);
        let base_font = descriptor
            .get::<Stream>(FONT_FILE2)
            .and_then(|s| s.decoded().ok())
            .and_then(|d| OpenTypeFontBlob::new(Arc::new(d.to_vec()), 0))?;

        let mut glyph_names = HashMap::new();

        // TODO: This is still pretty slow, see test file `font_truetype_slow_post_lookup`.
        if let Ok(post) = base_font.font_ref().post() {
            for i in 0..base_font.num_glyphs() {
                if let Some(str) = post.glyph_name(GlyphId16::new(i)) {
                    glyph_names.insert(str.to_string(), GlyphId::new(i as u32));
                }
            }
        }

        let cache_key = dict.cache_key();

        let cff_font_blob = base_font
            .font_ref()
            .cff()
            .ok()
            .and_then(|cff| CffFontBlob::new(Arc::new(cff.offset_data().as_ref().to_vec())));

        Some(Self {
            base_font,
            cache_key,
            differences,
            cff_blob: cff_font_blob,
            widths,
            glyph_names,
            font_flags,
            encoding,
            cached_mappings: RefCell::new(HashMap::new()),
        })
    }

    pub(crate) fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        self.base_font.outline_glyph(glyph)
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
    }

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
        if let Some(glyph) = self.cached_mappings.borrow().get(&code) {
            return *glyph;
        }

        if let Some(blob) = self.cff_blob.as_ref() {
            let table = blob.table();

            return self
                .code_to_name(code)
                .and_then(|name| table.glyph_index_by_name(name))
                .map(|g| GlyphId::new(g.0 as u32))
                .unwrap_or(GlyphId::NOTDEF);
        }

        let mut glyph = None;

        if self.is_non_symbolic() {
            let Some(lookup) = self.code_to_name(code) else {
                return GlyphId::NOTDEF;
            };

            if let Ok(cmap) = self.base_font.font_ref().cmap() {
                for record in cmap.encoding_records() {
                    if record.platform_id() == PlatformId::Windows
                        && record.encoding_id() == 1
                        && let Ok(subtable) = record.subtable(cmap.offset_data())
                    {
                        let convert = |input: &str| {
                            u32::from_str_radix(input, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .map(|s| s.to_string())
                        };

                        glyph = glyph.or_else(|| {
                            glyph_names::get(lookup)
                                .map(|n| n.to_string())
                                .or_else(|| {
                                    lookup
                                        .starts_with("uni")
                                        .then(|| lookup.get(3..).and_then(convert))
                                        .or_else(|| {
                                            lookup
                                                .starts_with("u")
                                                .then(|| lookup.get(1..).and_then(convert))
                                        })
                                        .flatten()
                                })
                                .and_then(|n| n.chars().next())
                                .and_then(|c| subtable.map_codepoint(c))
                                .filter(|g| *g != GlyphId::NOTDEF)
                        })
                    }
                }

                for record in cmap.encoding_records() {
                    if record.platform_id() == PlatformId::Macintosh
                        && record.encoding_id() == 0
                        && let Ok(subtable) = record.subtable(cmap.offset_data())
                    {
                        glyph = glyph.or_else(|| {
                            mac_os_roman::get_inverse(lookup)
                                .or_else(|| mac_roman::get_inverse(lookup))
                                .and_then(|c| subtable.map_codepoint(c))
                                .filter(|g| *g != GlyphId::NOTDEF)
                        })
                    }
                }
            }

            if glyph.is_none() {
                if let Some(gid) = self.glyph_names.get(lookup) {
                    glyph = Some(*gid);
                } else if let Some(gid) = glyph_num_string(lookup) {
                    glyph = Some(GlyphId::new(gid));
                }
            }
        } else if let Ok(cmap) = self.base_font.font_ref().cmap() {
            for record in cmap.encoding_records() {
                if record.platform_id() == PlatformId::Windows
                    && matches!(record.encoding_id(), 0 | 1)
                {
                    if let Ok(subtable) = record.subtable(cmap.offset_data()) {
                        for offset in [0x0000u32, 0xF000, 0xF100, 0xF200] {
                            glyph = glyph
                                .or_else(|| subtable.map_codepoint(code as u32 + offset))
                                .filter(|g| *g != GlyphId::NOTDEF)
                        }
                    }
                } else if matches!(
                    record.platform_id(),
                    PlatformId::Macintosh | PlatformId::Unicode
                ) && record.encoding_id() == 0
                    && let Ok(subtable) = record.subtable(cmap.offset_data())
                {
                    glyph = glyph
                        .or_else(|| subtable.map_codepoint(code))
                        .filter(|g| *g != GlyphId::NOTDEF)
                }
            }
        }

        let glyph = glyph.unwrap_or(GlyphId::NOTDEF);
        self.cached_mappings.borrow_mut().insert(code, glyph);

        glyph
    }

    pub(crate) fn glyph_width(&self, code: u8) -> f32 {
        self.widths
            .get(code as usize)
            .copied()
            .or_else(|| {
                self.base_font
                    .glyph_metrics()
                    .advance_width(self.map_code(code))
            })
            .warn_none(&format!("failed to find advance width for code {code}"))
            .unwrap_or(0.0)
    }
}

pub(crate) fn read_widths(dict: &Dict, descriptor: &Dict) -> Vec<f32> {
    let mut widths = Vec::new();

    let first_char = dict.get::<usize>(FIRST_CHAR);
    let last_char = dict.get::<usize>(LAST_CHAR);
    let widths_arr = dict.get::<Array>(WIDTHS);
    let missing_width = descriptor.get::<f32>(MISSING_WIDTH).unwrap_or(0.0);

    if let (Some(fc), Some(lc), Some(w)) = (first_char, last_char, widths_arr) {
        let iter = w.iter::<f32>().take(lc - fc + 1);

        for _ in 0..fc {
            widths.push(missing_width);
        }

        for w in iter {
            widths.push(w);
        }

        while widths.len() <= (u8::MAX as usize) + 1 {
            widths.push(missing_width);
        }
    }

    widths
}

fn glyph_num_string(s: &str) -> Option<u32> {
    if !s.starts_with('g') || s.len() < 2 {
        return None;
    }

    s[1..].parse::<u32>().ok()
}

impl CacheKey for TrueTypeFont {
    fn cache_key(&self) -> u128 {
        self.cache_key
    }
}

pub(crate) fn read_encoding(dict: &Dict) -> (Encoding, HashMap<u8, String>) {
    fn get_encoding_base(dict: &Dict, name: Name) -> Encoding {
        match dict.get::<Name>(name.clone()) {
            Some(n) => match n.deref() {
                WIN_ANSI_ENCODING => Encoding::WinAnsi,
                MAC_ROMAN_ENCODING => Encoding::MacRoman,
                MAC_EXPERT_ENCODING => Encoding::MacExpert,
                _ => {
                    warn!("Unknown font encoding {}", name.as_str());

                    Encoding::Standard
                }
            },
            None => Encoding::BuiltIn,
        }
    }

    let mut map = HashMap::new();

    if let Some(encoding_dict) = dict.get::<Dict>(ENCODING) {
        if let Some(differences) = encoding_dict.get::<Array>(DIFFERENCES) {
            let entries = differences.iter::<Object>();

            let mut code = 0;

            for obj in entries {
                if let Some(num) = obj.clone().into_i32() {
                    code = num;
                } else if let Some(name) = obj.into_name() {
                    map.insert(code as u8, name.as_str().to_string());
                    code += 1;
                }
            }
        }

        (
            get_encoding_base(&encoding_dict, Name::new(BASE_ENCODING)),
            map,
        )
    } else {
        (get_encoding_base(dict, Name::new(ENCODING)), HashMap::new())
    }
}
