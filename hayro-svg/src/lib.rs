/*!
A crate for converting PDF pages to SVG files.

This is the pendant to [`hayro`](https://crates.io/crates/hayro), but allows you to export to
SVG instead of bitmap images. See the description of that crate for more information on the
supported features and limitations.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use crate::clip::CachedClipPath;
use crate::glyph::{CachedOutlineGlyph, CachedType3Glyph};
use crate::mask::MaskKind;
use crate::paint::{CachedShading, CachedShadingPattern, CachedTilingPattern};
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::util::{Float32Ext, PageExt};
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterSettings,
    Paint, PathDrawMode, SoftMask, StrokeProps, interpret_page,
};
use kurbo::{Affine, BezPath, Cap, Join, Rect};
use siphasher::sip128::{Hasher128, SipHasher13};
use std::collections::HashMap;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::hash::Hash;
use xmlwriter::{Options, XmlWriter};

mod clip;
mod glyph;
pub(crate) mod image;
mod mask;
pub(crate) mod paint;
mod path;

/// Convert the given page into an SVG string.
pub fn convert(page: &Page, interpreter_settings: &InterpreterSettings) -> String {
    let mut state = Context::new(
        page.initial_transform(true),
        Rect::new(
            0.0,
            0.0,
            page.render_dimensions().0 as f64,
            page.render_dimensions().1 as f64,
        ),
        page.xref(),
        interpreter_settings.clone(),
    );
    let mut device = SvgRenderer::new(page);
    device.write_header(page.render_dimensions());

    interpret_page(page, &mut state, &mut device);

    device.finish()
}

pub(crate) struct SvgRenderer<'a> {
    pub(crate) xml: XmlWriter,
    pub(crate) outline_glyphs: Deduplicator<CachedOutlineGlyph>,
    pub(crate) type3_glyphs: Deduplicator<CachedType3Glyph<'a>>,
    pub(crate) clip_paths: Deduplicator<CachedClipPath>,
    pub(crate) masks: Deduplicator<MaskKind<'a>>,
    pub(crate) shadings: Deduplicator<CachedShading>,
    pub(crate) shading_patterns: Deduplicator<CachedShadingPattern>,
    pub(crate) tiling_patterns: Deduplicator<CachedTilingPattern<'a>>,
    pub(crate) dimensions: (f32, f32),
    pub(crate) cur_mask: Option<SoftMask<'a>>,
    pub(crate) cur_blend_mode: BlendMode,
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn write_transform(&mut self, transform: Affine) {
        let c = transform.as_coeffs();
        let has_scale = !(c[0] as f32).is_nearly_equal(1.0) || !(c[3] as f32).is_nearly_equal(1.0);
        let has_skew = !(c[1] as f32).is_nearly_equal(0.0) || !(c[2] as f32).is_nearly_equal(0.0);
        let has_translate =
            !(c[4] as f32).is_nearly_equal(0.0) || !(c[5] as f32).is_nearly_equal(0.0);
        let is_identity = !has_scale && !has_skew && !has_translate;

        if !is_identity {
            let transform = match (has_scale, has_skew, has_translate) {
                (true, false, false) => {
                    format!("scale({} {})", c[0] as f32, c[3] as f32)
                }
                (false, false, true) => {
                    format!("translate({} {})", c[4] as f32, c[5] as f32)
                }
                _ => {
                    format!("matrix({})", &convert_transform(&transform))
                }
            };

            self.xml.write_attribute("transform", &transform);
        }
    }

    fn push_transparency_group_inner(
        &mut self,
        opacity: f32,
        mask: Option<MaskKind<'a>>,
        blend_mode: BlendMode,
    ) {
        let mask_id = mask.map(|m| self.get_mask_id(m));

        self.xml.start_element("g");

        if let Some(mask_id) = mask_id {
            self.xml
                .write_attribute_fmt("mask", format_args!("url(#{mask_id})"));
        }

        if blend_mode != BlendMode::Normal {
            let bm_name = match blend_mode {
                BlendMode::Normal => "normal",
                BlendMode::Multiply => "multiply",
                BlendMode::Screen => "screen",
                BlendMode::Overlay => "overlay",
                BlendMode::Darken => "darken",
                BlendMode::Lighten => "lighten",
                BlendMode::ColorDodge => "color-dodge",
                BlendMode::ColorBurn => "color-burn",
                BlendMode::HardLight => "hard-light",
                BlendMode::SoftLight => "soft-light",
                BlendMode::Difference => "difference",
                BlendMode::Exclusion => "exclusion",
                BlendMode::Hue => "hue",
                BlendMode::Saturation => "saturation",
                BlendMode::Color => "color",
                BlendMode::Luminosity => "luminosity",
            };

            self.xml
                .write_attribute("style", &format!("mix-blend-mode:{}", bm_name));
        }

        if !opacity.is_nearly_equal(1.0) {
            self.xml.write_attribute("opacity", &opacity.to_string());
        }
    }

    pub(crate) fn write_stroke_properties(&mut self, stroke_props: &StrokeProps) {
        if !stroke_props.line_width.is_nearly_equal(1.0) {
            self.xml
                .write_attribute("stroke-width", &stroke_props.line_width)
        }

        match stroke_props.line_cap {
            Cap::Butt => {}
            Cap::Square => self.xml.write_attribute("stroke-linecap", "square"),
            Cap::Round => self.xml.write_attribute("stroke-linecap", "round"),
        }

        match stroke_props.line_join {
            Join::Bevel => self.xml.write_attribute("stroke-linejoin", "bevel"),
            Join::Miter => {}
            Join::Round => self.xml.write_attribute("stroke-linejoin", "round"),
        }

        if !stroke_props.miter_limit.is_nearly_equal(4.0) {
            self.xml
                .write_attribute("stroke-miterlimit", &stroke_props.miter_limit);
        }

        if !stroke_props.dash_offset.is_nearly_equal(0.0) {
            self.xml
                .write_attribute("stroke-dashoffset", &stroke_props.dash_offset);
        }

        if !stroke_props.dash_array.is_empty() {
            self.xml.write_attribute(
                "stroke-dasharray",
                &stroke_props
                    .dash_array
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<String>>()
                    .join(","),
            );
        }
    }

    fn with_group(&mut self, func: impl FnOnce(&mut SvgRenderer<'a>)) {
        let push_group = self.cur_mask.is_some() || self.cur_blend_mode != BlendMode::Normal;

        if push_group {
            self.push_transparency_group(1.0, self.cur_mask.clone(), self.cur_blend_mode);
        }

        func(self);

        if push_group {
            self.pop_transparency_group();
        }
    }
}

impl<'a> Device<'a> for SvgRenderer<'a> {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        self.cur_mask = mask;
    }

    fn set_blend_mode(&mut self, blend_mode: BlendMode) {
        self.cur_blend_mode = blend_mode;
    }

    fn draw_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &PathDrawMode,
    ) {
        self.with_group(|r| {
            Self::draw_path(r, path, transform, paint, draw_mode);
        })
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

    fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'a>>,
        blend_mode: BlendMode,
    ) {
        self.push_transparency_group_inner(opacity, mask.map(MaskKind::SoftMask), blend_mode);
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        self.with_group(|r| {
            Self::draw_glyph(r, glyph, transform, glyph_transform, paint, draw_mode);
        })
    }

    fn draw_image(&mut self, image: Image<'a, '_>, mut transform: Affine) {
        // TODO: Use Self::group
        match image {
            Image::Stencil(s) => {
                s.with_stencil(|s, paint| {
                    transform *= Affine::scale_non_uniform(
                        s.scale_factors.0 as f64,
                        s.scale_factors.1 as f64,
                    );
                    Self::draw_stencil_image(self, s, transform, paint);
                });
            }
            Image::Raster(r) => {
                r.with_rgba(|rgb, alpha| {
                    transform *= Affine::scale_non_uniform(
                        rgb.scale_factors.0 as f64,
                        rgb.scale_factors.1 as f64,
                    );
                    Self::draw_rgba_image(self, rgb, transform, alpha);
                });
            }
        }
    }

    fn pop_clip_path(&mut self) {
        self.xml.end_element();
    }

    fn pop_transparency_group(&mut self) {
        self.xml.end_element();
    }
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn new(page: &'a Page<'a>) -> Self {
        Self {
            xml: XmlWriter::new(Options::default()),
            outline_glyphs: Deduplicator::new('g'),
            type3_glyphs: Deduplicator::new('e'),
            clip_paths: Deduplicator::new('c'),
            masks: Deduplicator::new('m'),
            shadings: Deduplicator::new('s'),
            shading_patterns: Deduplicator::new('v'),
            tiling_patterns: Deduplicator::new('t'),
            cur_mask: None,
            dimensions: page.render_dimensions(),
            cur_blend_mode: Default::default(),
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

    // We need this because we have a small problem. `xmlwriter` doesn't allow us to write sub-streams
    // of XML while we are writing our main stream. This means that objects that need to be interpreted
    // (like patterns or mask) all need to be written to the XML in the end. On the other hand, once
    // we get to `finish` all of the registerd resources must already have been registered. This isn't
    // the case if masks or patterns use new resources that haven't been registered before. As a result,
    pub(crate) fn with_dummy(&mut self, f: impl FnOnce(&mut Self)) {
        let mut old_xml = std::mem::replace(&mut self.xml, XmlWriter::new(Options::default()));
        f(self);
        std::mem::swap(&mut self.xml, &mut old_xml);
    }

    pub(crate) fn finish(mut self) -> String {
        self.write_glyph_defs();
        self.write_mask_defs();
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

    pub(crate) fn contains(&self, hash: u128) -> bool {
        self.present.contains_key(&hash)
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

    pub(crate) fn insert(&mut self, value: T) -> Id {
        let index = self.vec.len();
        self.vec.push(value);
        Id(self.kind, index as u64)
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

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct Id(char, u64);

impl Display for Id {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.0, self.1)
    }
}

pub(crate) fn hash128<T: Hash + ?Sized>(value: &T) -> u128 {
    let mut state = SipHasher13::new();
    value.hash(&mut state);
    state.finish128().as_u128()
}
