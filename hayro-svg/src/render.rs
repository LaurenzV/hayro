use crate::clip::CachedClipPath;
use crate::glyph::CachedGlyph;
use crate::paint::{CachedShading, CachedShadingPattern, CachedTilingPattern};
use crate::{Id, hash128};
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::{
    CacheKey, ClipPath, Device, FillRule, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use kurbo::{Affine, BezPath, PathEl};
use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::marker::PhantomData;
use xmlwriter::{Options, XmlWriter};

pub(crate) struct SvgRenderer<'a> {
    pub(crate) xml: XmlWriter,
    pub(crate) glyphs: Deduplicator<CachedGlyph>,
    pub(crate) clip_paths: Deduplicator<CachedClipPath>,
    pub(crate) shadings: Deduplicator<CachedShading>,
    pub(crate) shading_patterns: Deduplicator<CachedShadingPattern>,
    pub(crate) tiling_patterns: Deduplicator<CachedTilingPattern<'a>>,
    pub(crate) phantom_data: PhantomData<&'a ()>,
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn write_transform(&mut self, transform: Affine) {
        let is_identity = {
            let c = transform.as_coeffs();
            c[0] == 1.0 && c[1] == 0.0 && c[2] == 0.0 && c[3] == 1.0 && c[4] == 0.0 && c[5] == 0.0
        };

        if !is_identity {
            self.xml.write_attribute(
                "transform",
                &format!("matrix({})", &convert_transform(&transform)),
            );
        }
    }
}

impl<'a> Device<'a> for SvgRenderer<'a> {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}

    fn draw_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &PathDrawMode,
    ) {
        Self::draw_path(self, path, transform, paint, draw_mode);
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        Self::draw_glyph(self, glyph, transform, glyph_transform, paint, draw_mode);
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        let clip_id = self
            .clip_paths
            .insert_with(clip_path.cache_key(), || CachedClipPath {
                path: clip_path.path.clone(),
                fill_rule: clip_path.fill,
            });

        self.xml.start_element("g");
        self.xml
            .write_attribute_fmt("clip-path", format_args!("url(#{clip_id})"));
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>) {}

    fn pop_clip_path(&mut self) {
        self.xml.end_element();
    }

    fn pop_transparency_group(&mut self) {}

    fn draw_image(&mut self, image: Image<'_>, transform: Affine) {
        match image {
            Image::Stencil(s) => {
                s.with_stencil(|s, paint| {
                    Self::draw_stencil_image(self, s, transform, paint);
                });
            }
            Image::Raster(r) => {
                r.with_rgba(|rgb, alpha| {
                    Self::draw_rgba_image(self, rgb, transform, alpha);
                });
            }
        }
    }
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn new(_: &'a Page<'a>) -> Self {
        Self {
            xml: XmlWriter::new(Options::default()),
            glyphs: Deduplicator::new('g'),
            clip_paths: Deduplicator::new('c'),
            shadings: Deduplicator::new('s'),
            shading_patterns: Deduplicator::new('v'),
            tiling_patterns: Deduplicator::new('t'),
            phantom_data: PhantomData,
        }
    }

    pub(crate) fn write_header(&mut self, size: (f32, f32)) {
        self.xml.start_element("svg");
        self.xml
            .write_attribute_fmt("viewBox", format_args!("0 0 {} {}", size.0, size.1));
        self.xml
            .write_attribute_fmt("width", format_args!("{}", size.0));
        self.xml
            .write_attribute_fmt("height", format_args!("{}", size.1));
        self.xml
            .write_attribute("xmlns", "http://www.w3.org/2000/svg");
        self.xml
            .write_attribute("xmlns:xlink", "http://www.w3.org/1999/xlink");
    }

    pub(crate) fn finish(mut self) -> String {
        let mut old_xml = std::mem::replace(&mut self.xml, XmlWriter::new(Options::default()));
        self.write_tiling_pattern_defs();
        std::mem::swap(&mut self.xml, &mut old_xml);

        self.write_glyph_defs();
        self.write_clip_path_defs();
        self.write_shading_defs();
        self.write_shading_pattern_defs();
        self.write_tiling_pattern_defs();
        // Close the `svg` element.
        self.xml.end_element();
        self.xml.end_document()
    }
}

pub(crate) fn convert_transform(transform: &Affine) -> String {
    transform
        .as_coeffs()
        .iter()
        .map(|c| (*c as f32).to_string())
        .collect::<Vec<String>>()
        .join(" ")
}

#[derive(Debug, Clone)]
pub(crate) struct Deduplicator<T> {
    kind: char,
    vec: Vec<T>,
    present: HashMap<u128, Id>,
}

impl<T> Default for Deduplicator<T> {
    fn default() -> Self {
        Self::new('-')
    }
}

impl<T> Deduplicator<T> {
    fn new(kind: char) -> Self {
        Self {
            kind,
            vec: Vec::new(),
            present: HashMap::new(),
        }
    }

    pub(crate) fn insert_with<F>(&mut self, hash: u128, f: F) -> Id
    where
        F: FnOnce() -> T,
    {
        *self.present.entry(hash).or_insert_with(|| {
            let index = self.vec.len();
            self.vec.push(f());
            Id(self.kind, index as u64)
        })
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (Id, &T)> {
        self.vec
            .iter()
            .enumerate()
            .map(|(i, v)| (Id(self.kind, i as u64), v))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }
}
