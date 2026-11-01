use crate::GlyphId;
use crate::program::cff::CffProgram;
use crate::program::content_hash;
use crate::program::pen::OutlinePath;
use crate::{FontData, UNITS_PER_EM};
use kurbo::BezPath;
use rustc_hash::FxHashMap;
use skrifa::attribute::Style;
use skrifa::instance::{LocationRef, Size};
use skrifa::metrics::GlyphMetrics;
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, Target};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::cmap::{self, PlatformId};
use skrifa::raw::tables::post::DEFAULT_GLYPH_NAMES;
use skrifa::raw::types::Tag;
use skrifa::{FontRef, MetadataProvider, OutlineGlyphCollection};
use std::fmt::{Debug, Formatter};
use std::ops::Range;
use std::sync::{Arc, OnceLock};
use yoke::{Yoke, Yokeable};

#[derive(Clone)]
pub(crate) struct OpenTypeProgram(Arc<Inner>);

impl OpenTypeProgram {
    pub(crate) fn new(data: FontData, index: u32) -> Option<Self> {
        let hash = content_hash(data.as_ref().as_ref(), index);
        let yoke = Yoke::<OpenTypeYoke<'static>, FontData>::try_attach_to_cart(data, |data| {
            let font_ref = FontRef::from_index(data.as_ref(), index).map_err(|_| ())?;

            // Reject fonts with invalid post table version, fixes pdf.js issue 9462. Not sure
            // if there is a better fix, for some reason skrifa accepts the font which is
            // completely invalid.
            if font_ref.post().is_ok_and(|p| {
                !matches!(
                    p.version().to_major_minor(),
                    (1, 0) | (2, 0) | (2, 5) | (3, 0)
                )
            }) {
                return Err(());
            }

            // Some fonts render wrongly without their instructions, so they are
            // always hinted, at the size `UNITS_PER_EM`.
            let hinting_instance = if font_ref.outline_glyphs().require_interpreter() {
                HintingInstance::new(
                    &font_ref.outline_glyphs(),
                    Size::new(UNITS_PER_EM),
                    LocationRef::default(),
                    HintingOptions {
                        engine: Engine::Interpreter,
                        target: Target::Mono,
                    },
                )
                .ok()
            } else {
                None
            };

            Ok(OpenTypeYoke {
                font_ref: font_ref.clone(),
                outline_glyphs: font_ref.outline_glyphs(),
                hinting_instance,
                glyph_metrics: font_ref
                    .glyph_metrics(Size::new(UNITS_PER_EM), LocationRef::default()),
            })
        })
        .ok()?;

        let cff = yoke
            .get()
            .font_ref
            .table_directory()
            .table_records()
            .iter()
            .find(|record| record.tag() == Tag::new(b"CFF "))
            .and_then(|record| {
                let start = record.offset() as usize;
                let range = start..start.checked_add(record.length() as usize)?;
                let data = TableData {
                    font: yoke.backing_cart().clone(),
                    range,
                };
                data.font.as_ref().as_ref().get(data.range.clone())?;
                CffProgram::with_hash(Arc::new(data), hash)
            });

        Some(Self(Arc::new(Inner {
            hash,
            yoke,
            cff,
            name_to_glyph: OnceLock::new(),
        })))
    }

    fn get(&self) -> &OpenTypeYoke<'_> {
        self.0.yoke.get()
    }

    pub(crate) fn content_hash(&self) -> u128 {
        self.0.hash
    }

    pub(crate) fn data(&self) -> &FontData {
        self.0.yoke.backing_cart()
    }

    fn font_ref(&self) -> &FontRef<'_> {
        &self.get().font_ref
    }

    pub(crate) fn is_cff(&self) -> bool {
        self.0.cff.is_some()
    }

    pub(crate) fn glyph_by_name(&self, name: &str) -> Option<GlyphId> {
        self.0
            .cff
            .as_ref()
            .and_then(|cff| cff.glyph_by_name(name))
            .or_else(|| {
                self.0
                    .name_to_glyph
                    .get_or_init(|| self.post_glyph_names())
                    .get(name)
                    .copied()
            })
    }

    pub(crate) fn builtin_encoding_glyph(&self, code: u8) -> Option<GlyphId> {
        self.0.cff.as_ref()?.builtin_encoding_glyph(code)
    }

    pub(crate) fn builtin_encoding_glyph_name(&self, code: u8) -> Option<&str> {
        self.0.cff.as_ref()?.builtin_encoding_glyph_name(code)
    }

    fn post_glyph_names(&self) -> FxHashMap<String, GlyphId> {
        // Note: We don't call the `glyph_name` method provided by read-fonts because
        // calling it repeatedly is very slow.
        let mut glyph_names = FxHashMap::default();
        let Ok(post) = self.font_ref().post() else {
            return glyph_names;
        };

        match post.version().to_major_minor() {
            (1, 0) => {
                for (gid, name) in DEFAULT_GLYPH_NAMES.iter().enumerate() {
                    glyph_names
                        .entry((*name).to_string())
                        .or_insert(GlyphId::new(gid as u32));
                }
            }
            (2, 0) => {
                let Some(name_indices) = post.glyph_name_index() else {
                    return glyph_names;
                };
                let Some(string_data) = post.string_data() else {
                    return glyph_names;
                };
                let custom_names = string_data
                    .iter()
                    .map(|entry| entry.ok().map(|name| name.as_str()))
                    .collect::<Vec<_>>();

                for (gid, idx) in name_indices.iter().enumerate() {
                    let idx = idx.get() as usize;
                    if let Some(name) = DEFAULT_GLYPH_NAMES.get(idx).copied().or_else(|| {
                        custom_names
                            .get(idx.saturating_sub(DEFAULT_GLYPH_NAMES.len()))
                            .copied()
                            .flatten()
                    }) {
                        glyph_names
                            .entry(name.to_string())
                            .or_insert(GlyphId::new(gid as u32));
                    }
                }
            }
            _ => {}
        }

        glyph_names
    }

    pub(crate) fn cmap_glyph(&self, c: char) -> Option<GlyphId> {
        self.font_ref().charmap().map(c).map(GlyphId::from_skrifa)
    }

    pub(crate) fn cmap_subtables(&self) -> Vec<CmapSubtable<'_>> {
        let Ok(cmap) = self.font_ref().cmap() else {
            return Vec::new();
        };

        cmap.encoding_records()
            .iter()
            .filter_map(|record| {
                Some(CmapSubtable {
                    platform: match record.platform_id() {
                        PlatformId::Unicode => CmapPlatform::Unicode,
                        PlatformId::Macintosh => CmapPlatform::Macintosh,
                        PlatformId::Windows => CmapPlatform::Windows,
                        _ => CmapPlatform::Other,
                    },
                    encoding_id: record.encoding_id(),
                    subtable: record.subtable(cmap.offset_data()).ok()?,
                })
            })
            .collect()
    }

    pub(crate) fn advance_width(&self, glyph: GlyphId) -> Option<f32> {
        self.get().glyph_metrics.advance_width(glyph.to_skrifa())
    }

    pub(crate) fn weight(&self) -> Option<u32> {
        let weight = self.font_ref().attributes().weight.value().round() as u32;
        (weight > 0).then_some(weight)
    }

    pub(crate) fn is_italic(&self) -> bool {
        self.font_ref().attributes().style != Style::Normal
    }

    pub(crate) fn is_monospace(&self) -> bool {
        self.font_ref()
            .metrics(Size::unscaled(), LocationRef::default())
            .is_monospace
    }

    pub(crate) fn outline(&self, glyph: GlyphId) -> BezPath {
        if let Some(cff) = &self.0.cff {
            return cff.outline(glyph);
        }

        let mut path = OutlinePath::new();

        let draw_settings = if let Some(instance) = self.get().hinting_instance.as_ref() {
            DrawSettings::hinted(instance, false)
        } else {
            DrawSettings::unhinted(Size::new(UNITS_PER_EM), LocationRef::default())
        };

        let Some(outline) = self.get().outline_glyphs.get(glyph.to_skrifa()) else {
            return BezPath::new();
        };

        let _ = outline.draw(draw_settings, &mut path);
        path.take()
    }
}

impl Debug for OpenTypeProgram {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpenTypeProgram {{ .. }}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CmapPlatform {
    Unicode,
    Macintosh,
    Windows,
    Other,
}

pub(crate) struct CmapSubtable<'a> {
    platform: CmapPlatform,
    encoding_id: u16,
    subtable: cmap::CmapSubtable<'a>,
}

impl CmapSubtable<'_> {
    pub(crate) fn platform(&self) -> CmapPlatform {
        self.platform
    }

    pub(crate) fn encoding_id(&self) -> u16 {
        self.encoding_id
    }

    pub(crate) fn map(&self, code: u32) -> Option<GlyphId> {
        self.subtable.map_codepoint(code).map(GlyphId::from_skrifa)
    }
}

struct Inner {
    hash: u128,
    yoke: Yoke<OpenTypeYoke<'static>, FontData>,
    // Stored separately so that the subfonts of the CFF table can be cached,
    // which isn't possible using the current skrifa API.
    cff: Option<CffProgram>,
    name_to_glyph: OnceLock<FxHashMap<String, GlyphId>>,
}

#[derive(Yokeable)]
struct OpenTypeYoke<'a> {
    font_ref: FontRef<'a>,
    glyph_metrics: GlyphMetrics<'a>,
    hinting_instance: Option<HintingInstance>,
    outline_glyphs: OutlineGlyphCollection<'a>,
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
