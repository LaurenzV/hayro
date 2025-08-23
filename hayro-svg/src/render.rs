use crate::paint::{CachedShading, CachedShadingPattern, CachedTilingPattern};
use crate::{Id, hash128};
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::{
    CacheKey, ClipPath, Device, FillRule, GlyphDrawMode, LumaData, Paint, PathDrawMode, RgbData,
    SoftMask,
};
use kurbo::{Affine, BezPath, PathEl};
use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::marker::PhantomData;
use xmlwriter::{Options, XmlWriter};

struct CachedClipPath {
    path: BezPath,
    fill_rule: FillRule,
}

struct CachedGlyph {
    path: BezPath,
}

pub(crate) struct SvgRenderer<'a> {
    pub(crate) xml: XmlWriter,
    glyphs: Deduplicator<CachedGlyph>,
    clip_paths: Deduplicator<CachedClipPath>,
    pub(crate) shadings: Deduplicator<CachedShading>,
    pub(crate) shading_patterns: Deduplicator<CachedShadingPattern>,
    pub(crate) tiling_patterns: Deduplicator<CachedTilingPattern<'a>>,
    pub(crate) phantom_data: PhantomData<&'a ()>,
}

impl<'a> SvgRenderer<'a> {
    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        mode: &GlyphDrawMode,
    ) {
        match glyph {
            Glyph::Outline(o) => {
                let outline = o.outline();
                let cache_key = hash128(&(o.identifier().cache_key(), glyph_transform.cache_key()));
                let id = self.glyphs.insert_with(cache_key, || CachedGlyph {
                    path: glyph_transform * outline.clone(),
                });

                self.xml.start_element("use");
                self.xml
                    .write_attribute_fmt("xlink:href", format_args!("#{id}"));
                self.write_transform(transform);

                match mode {
                    GlyphDrawMode::Fill => {
                        self.write_paint(paint, &outline, transform, false);
                    }
                    GlyphDrawMode::Stroke(_) => {
                        self.write_paint(paint, &outline, transform, true);
                    }
                }
                self.xml.end_element();
            }
            Glyph::Type3(_) => {}
        }
    }

    fn draw_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &PathDrawMode,
    ) {
        let svg_path = path.to_svg_f32();

        self.xml.start_element("path");
        self.xml.write_attribute("d", &svg_path);

        match draw_mode {
            PathDrawMode::Fill(_) => {
                self.write_paint(paint, path, transform, false);
            }
            PathDrawMode::Stroke(_) => {
                self.write_paint(paint, path, transform, true);
            }
        }

        self.write_transform(transform);
        self.xml.end_element();
    }

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

    fn write_glyph_defs(&mut self) {
        if self.glyphs.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "glyph");

        for (id, glyph) in self.glyphs.iter() {
            self.xml.start_element("path");
            self.xml.write_attribute("id", &id);
            self.xml.write_attribute("d", &glyph.path.to_svg_f32());
            self.xml.end_element();
        }

        self.xml.end_element();
    }

    fn write_clip_path_defs(&mut self) {
        if self.clip_paths.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "clip-path");

        for (id, clip_path) in self.clip_paths.iter() {
            self.xml.start_element("clipPath");
            self.xml.write_attribute("id", &id);
            self.xml.start_element("path");
            self.xml.write_attribute("d", &clip_path.path.to_svg_f32());

            if clip_path.fill_rule == FillRule::EvenOdd {
                self.xml.write_attribute("clip-rule", "evenodd");
            }

            self.xml.end_element();
            self.xml.end_element();
        }

        self.xml.end_element();
    }

    fn insert_clip(&mut self, clip_path: &ClipPath) -> Id {
        self.clip_paths
            .insert_with(clip_path.cache_key(), || CachedClipPath {
                path: clip_path.path.clone(),
                fill_rule: clip_path.fill,
            })
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
        let clip_id = self.insert_clip(clip_path);

        self.xml.start_element("g");
        self.xml
            .write_attribute_fmt("clip-path", format_args!("url(#{clip_id})"));
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>) {}

    fn draw_rgba_image(&mut self, image: RgbData, transform: Affine, alpha: Option<LumaData>) {
        Self::draw_rgba_image(self, image, transform, alpha);
    }

    fn draw_stencil_image(&mut self, stencil: LumaData, transform: Affine, paint: &Paint) {
        Self::draw_stencil_image(self, stencil, transform, paint);
    }

    fn pop_clip_path(&mut self) {
        self.xml.end_element();
    }

    fn pop_transparency_group(&mut self) {}
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

trait BezPathExt {
    fn to_svg_f32(&self) -> String {
        let mut buffer = Vec::new();
        self.write_to_f32(&mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    fn write_to_f32<W: Write>(&self, writer: W) -> io::Result<()>;
}

impl BezPathExt for BezPath {
    fn to_svg_f32(&self) -> String {
        let mut buffer = Vec::new();
        self.write_to_f32(&mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    /// Write the SVG representation of this path to the provided buffer.
    fn write_to_f32<W: Write>(&self, mut writer: W) -> io::Result<()> {
        for (i, el) in self.elements().iter().enumerate() {
            if i > 0 {
                write!(writer, " ")?;
            }
            match *el {
                PathEl::MoveTo(p) => write!(writer, "M{},{}", p.x as f32, p.y as f32)?,
                PathEl::LineTo(p) => write!(writer, "L{},{}", p.x as f32, p.y as f32)?,
                PathEl::QuadTo(p1, p2) => write!(
                    writer,
                    "Q{},{} {},{}",
                    p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32
                )?,
                PathEl::CurveTo(p1, p2, p3) => write!(
                    writer,
                    "C{},{} {},{} {},{}",
                    p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32, p3.x as f32, p3.y as f32
                )?,
                PathEl::ClosePath => write!(writer, "Z")?,
            }
        }

        Ok(())
    }
}
