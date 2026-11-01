use crate::FontData;
use crate::GlyphId;
use crate::{HintingCache, UNITS_PER_EM};
use kurbo::{Affine, BezPath};
use siphasher::sip128::{Hasher128, SipHasher13};
use skrifa::font::Kind;
use skrifa::raw::model::attribute::Style;
use skrifa::raw::model::charmap::Encoding;
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{
    DrawMemory, Font, GlyphSource, GlyphSourceOrder, HintOptions, OutlineSource, RasterTarget,
    ScalerFont, Tag,
};
use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

mod pen;

use pen::OutlinePath;

#[derive(Clone)]
pub(crate) struct FontProgram(Arc<Inner>);

impl FontProgram {
    pub(crate) fn new(data: FontData, index: u32) -> Option<Self> {
        let mut font = Font::new(data.clone(), index)?;

        // OpenType fonts embedded in PDFs sometimes only contain the tables needed for their
        // CFF outlines and lack `maxp`, which skrifa needs to know the number of glyphs.
        if matches!(font.kind(), Kind::Sfnt(..))
            && font.num_glyphs() == 0
            && let Some(cff) = cff_table_font(&data, index)
        {
            font = cff;
        }

        // Reject fonts with invalid post table version, fixes pdf.js issue 9462. Not sure
        // if there is a better fix, for some reason skrifa accepts the font which is
        // completely invalid.
        if matches!(font.kind(), Kind::Sfnt(..))
            && font.tables().post().is_ok_and(|p| {
                !matches!(
                    p.version().to_major_minor(),
                    (1, 0) | (2, 0) | (2, 5) | (3, 0)
                )
            })
        {
            return None;
        }

        let scaler = ScalerFont::unhinted(font.clone(), Some(UNITS_PER_EM));

        Some(Self(Arc::new(Inner {
            hash: content_hash(data.as_ref().as_ref(), index),
            data,
            font,
            scaler,
        })))
    }

    pub(crate) fn kind(&self) -> ProgramKind {
        match self.font().kind() {
            Kind::Sfnt(..) => ProgramKind::Sfnt,
            Kind::Cff(..) => ProgramKind::Cff,
            Kind::Type1(_) => ProgramKind::Type1,
        }
    }

    pub(crate) fn content_hash(&self) -> u128 {
        self.0.hash
    }

    pub(crate) fn data(&self) -> &FontData {
        &self.0.data
    }

    pub(crate) fn has_cff_outlines(&self) -> bool {
        self.cff().is_some()
    }

    pub(crate) fn is_cid_keyed(&self) -> bool {
        self.font().is_cid_keyed()
    }

    pub(crate) fn glyph_by_name(&self, name: &str) -> Option<GlyphId> {
        self.font().glyph_by_name(name).map(GlyphId::from_skrifa)
    }

    pub(crate) fn glyph_by_cid(&self, cid: u32) -> Option<GlyphId> {
        self.font().glyph_by_cid(cid).map(GlyphId::from_skrifa)
    }

    pub(crate) fn builtin_encoding_glyph(&self, code: u8) -> Option<GlyphId> {
        let font = self.font();
        let glyph = match font.kind() {
            Kind::Type1(t) => t.encoding()?.map(code),
            Kind::Cff(cff, _) => cff.encoding()?.map(code),
            Kind::Sfnt(..) => self.cff()?.encoding()?.map(code),
        };

        glyph.map(GlyphId::from_skrifa)
    }

    pub(crate) fn builtin_encoding_glyph_name(&self, code: u8) -> Option<&str> {
        let font = self.font();
        match font.kind() {
            Kind::Type1(t) => t.encoding()?.glyph_name(code),
            Kind::Cff(cff, _) => cff_glyph_name(&cff, cff.encoding()?.map(code)?),
            Kind::Sfnt(..) => {
                let cff = self.cff()?;
                cff_glyph_name(cff, cff.encoding()?.map(code)?)
            }
        }
    }

    pub(crate) fn cmap_glyph(&self, c: char) -> Option<GlyphId> {
        self.font()
            .charmap()
            .map_unicode(c)
            .map(GlyphId::from_skrifa)
    }

    pub(crate) fn cmap_subtables(&self) -> Vec<CmapSubtable<'_>> {
        let charmap = self.font().charmap();
        charmap
            .encodings()
            .filter_map(|encoding| {
                let (platform, encoding_id) = encoding.sfnt_ids()?;
                Some(CmapSubtable {
                    font: self.font(),
                    platform: match platform {
                        0 => CmapPlatform::Unicode,
                        1 => CmapPlatform::Macintosh,
                        3 => CmapPlatform::Windows,
                        _ => CmapPlatform::Other,
                    },
                    encoding_id,
                    encoding,
                })
            })
            .collect()
    }

    pub(crate) fn advance_width(&self, glyph: GlyphId) -> Option<f32> {
        let font = self.sfnt()?;
        let advance = font.glyph_metrics().h_advance(glyph.to_skrifa());

        Some(advance * UNITS_PER_EM / f32::from(font.units_per_em()))
    }

    pub(crate) fn weight(&self) -> Option<u32> {
        let weight = self.sfnt()?.default_attributes().weight.value().round() as u32;
        (weight > 0).then_some(weight)
    }

    pub(crate) fn is_italic(&self) -> bool {
        self.sfnt()
            .is_some_and(|font| font.default_attributes().style != Style::Normal)
    }

    pub(crate) fn is_monospace(&self) -> bool {
        self.sfnt()
            .and_then(|font| font.style_metrics().is_fixed_pitch)
            .unwrap_or(false)
    }

    pub(crate) fn hinted_outline(
        &self,
        glyph: GlyphId,
        ppem: f32,
        cache: &HintingCache,
    ) -> Option<BezPath> {
        // Fonts that need their instructions are drawn by `outline`, which runs them.
        if self.font().is_hint_reliant() {
            return None;
        }

        let scaler = cache.get_or_insert_with(self.content_hash(), ppem, || {
            // Like FreeType's light hinting, TrueType outlines use the auto-hinter.
            let options = match self.font().outline_glyphs().source() {
                Some(OutlineSource::Glyf) => HintOptions::auto(),
                _ => HintOptions::interpreter(),
            };
            let scaler = ScalerFont::hinted(
                self.font().clone(),
                ppem,
                options.with_target(RasterTarget::Light),
            );

            scaler.is_hinted().then_some(scaler)
        })?;

        let mut path = OutlinePath::new();
        scaler
            .outline(glyph.to_skrifa())?
            .draw(DrawMemory::Scratch, &mut path)
            .ok()?;

        Some(Affine::scale(f64::from(UNITS_PER_EM / ppem)) * path.take())
    }

    pub(crate) fn outline(&self, glyph: GlyphId) -> BezPath {
        let mut path = OutlinePath::new();
        if let Some(outline) = self.0.scaler.outline(glyph.to_skrifa()) {
            let _ = outline.draw(DrawMemory::Scratch, &mut path);
        }

        path.take()
    }

    fn font(&self) -> &Font {
        &self.0.font
    }

    fn cff(&self) -> Option<&CffFontRef<'_>> {
        self.font()
            .outline_glyphs_with(GlyphSourceOrder::new(&[GlyphSource::Cff]))
            .cff_font()
    }

    fn sfnt(&self) -> Option<&Font> {
        matches!(self.font().kind(), Kind::Sfnt(..)).then(|| self.font())
    }
}

impl Debug for FontProgram {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "FontProgram {{ .. }}")
    }
}

impl PartialEq for FontProgram {
    fn eq(&self, other: &Self) -> bool {
        self.content_hash() == other.content_hash()
    }
}

impl Eq for FontProgram {}

impl Hash for FontProgram {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u128(self.content_hash());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProgramKind {
    Sfnt,
    Cff,
    Type1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CmapPlatform {
    Unicode,
    Macintosh,
    Windows,
    Other,
}

pub(crate) struct CmapSubtable<'a> {
    font: &'a Font,
    platform: CmapPlatform,
    encoding_id: u16,
    encoding: Encoding,
}

impl CmapSubtable<'_> {
    pub(crate) fn platform(&self) -> CmapPlatform {
        self.platform
    }

    pub(crate) fn encoding_id(&self) -> u16 {
        self.encoding_id
    }

    pub(crate) fn map(&self, code: u32) -> Option<GlyphId> {
        self.font
            .charmap()
            .map_code(self.encoding, code)
            .map(GlyphId::from_skrifa)
    }
}

struct Inner {
    hash: u128,
    data: FontData,
    font: Font,
    scaler: ScalerFont,
}

fn cff_table_font(data: &FontData, index: u32) -> Option<Font> {
    let bytes = data.as_ref().as_ref();
    let record = *FontRef::from_index(bytes, index)
        .ok()?
        .table_directory()
        .table_records()
        .iter()
        .find(|record| record.tag() == Tag::new(b"CFF "))?;
    let start = record.offset() as usize;
    let range = start..start.checked_add(record.length() as usize)?;
    bytes.get(range.clone())?;

    let table: FontData = Arc::new(TableData {
        font: data.clone(),
        range,
    });
    Font::new(table, 0)
}

struct TableData {
    font: FontData,
    range: Range<usize>,
}

impl AsRef<[u8]> for TableData {
    fn as_ref(&self) -> &[u8] {
        &self.font.as_ref().as_ref()[self.range.clone()]
    }
}

fn cff_glyph_name<'a>(cff: &CffFontRef<'a>, glyph: skrifa::GlyphId) -> Option<&'a str> {
    if cff.is_cid() {
        return None;
    }

    let sid = cff.charset()?.string_id(glyph)?;
    std::str::from_utf8(cff.string(sid)?).ok()
}

pub(crate) fn content_hash(data: &[u8], index: u32) -> u128 {
    let mut state = SipHasher13::new();
    (data, index).hash(&mut state);
    state.finish128().as_u128()
}
