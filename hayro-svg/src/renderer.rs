use base64::Engine;
use hayro_interpret::color::Color;
use hayro_interpret::encode::EncodedShadingPattern;
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::pattern::{Pattern, ShadingPattern, TilingPattern};
use hayro_interpret::{
    CacheKey, ClipPath, Device, FillRule, LumaData, Paint, RgbData, SoftMask, StrokeProps,
};
use image::{DynamicImage, ImageBuffer, ImageFormat};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape, Vec2};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::io::{Cursor, Write};
use std::marker::PhantomData;
use std::{fmt, io};
use xmlwriter::{Options, XmlWriter};

struct CachedClipPath {
    path: BezPath,
    fill_rule: FillRule,
}

struct CachedShadingPattern {
    transform: Affine,
    shading: Id,
    bbox: Rect,
}

#[derive(Clone)]
struct CachedTilingPattern<'a> {
    transform: Affine,
    tiling_pattern: TilingPattern<'a>,
}

struct CachedShading {
    pattern: ShadingPattern,
    bbox: Rect,
}

pub(crate) struct SvgRenderer<'a> {
    xml: XmlWriter,
    transform: Affine,
    fill_rule: FillRule,
    stroke_props: StrokeProps,
    glyphs: Deduplicator<BezPath>,
    clip_paths: Deduplicator<CachedClipPath>,
    shadings: Deduplicator<CachedShading>,
    shading_patterns: Deduplicator<CachedShadingPattern>,
    tiling_patterns: Deduplicator<CachedTilingPattern<'a>>,
    phantom_data: PhantomData<&'a ()>,
}

impl<'a> SvgRenderer<'a> {
    fn fill_path(&mut self, path: &BezPath, paint: &Paint<'a>) {
        let svg_path = path.to_svg_f32();

        match &paint {
            Paint::Color(c) => {
                self.xml.start_element("path");
                self.xml.write_attribute("d", &svg_path);
                self.write_color(c, false);
                self.write_transform(None);
                self.xml.end_element();
            }
            Paint::Pattern(p) => match p.as_ref() {
                Pattern::Shading(s) => {
                    let bbox = (self.transform * path).bounding_box();
                    let shading_id = self.shadings.insert_with(s.cache_key(), || CachedShading {
                        pattern: s.clone(),
                        bbox,
                    });

                    let inverse_transform = self.transform.inverse();
                    let pattern_id = self.shading_patterns.insert_with(
                        (s.clone(), inverse_transform).cache_key(),
                        || CachedShadingPattern {
                            transform: inverse_transform,
                            bbox,
                            shading: shading_id,
                        },
                    );

                    self.xml.start_element("path");
                    self.xml.write_attribute("d", &svg_path);
                    self.xml
                        .write_attribute_fmt("fill", format_args!("url(#{pattern_id})"));
                    self.write_transform(None);
                    self.xml.end_element();
                }
                Pattern::Tiling(t) => {
                    let inverse_transform = self.transform.inverse();
                    let pattern = *t.clone();

                    let pattern_id = self.tiling_patterns.insert_with(
                        (pattern.clone(), inverse_transform).cache_key(),
                        || CachedTilingPattern {
                            transform: inverse_transform,
                            tiling_pattern: pattern,
                        },
                    );

                    self.xml.start_element("path");
                    self.xml.write_attribute("d", &svg_path);
                    self.xml
                        .write_attribute_fmt("fill", format_args!("url(#{pattern_id})"));
                    self.write_transform(None);
                    self.xml.end_element();
                }
            },
        }
    }

    fn write_color(&mut self, color: &Color, is_stroke: bool) {
        let (fill, alpha) = convert_color(color);

        if is_stroke {
            self.xml.write_attribute("stroke", &fill);
            if alpha != 1.0 {
                self.xml.write_attribute("stroke-opacity", &alpha);
            }
        } else {
            self.xml.write_attribute("fill", &fill);

            if alpha != 1.0 {
                self.xml.write_attribute("fill-opacity", &alpha);
            }
        }
    }

    fn stroke_path(&mut self, path: &BezPath, paint: &Paint) {
        let svg_path = path.to_svg_f32();

        match &paint {
            Paint::Color(c) => {
                self.xml.start_element("path");
                self.xml.write_attribute("d", &svg_path);
                self.write_color(c, true);
                self.xml.write_attribute("fill", "none");
                self.write_transform(None);
                self.xml.end_element();
            }
            Paint::Pattern(_) => {
                unimplemented!();
            }
        }
    }

    fn write_transform(&mut self, transform: Option<Affine>) {
        let transform = transform.unwrap_or(self.transform);
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

    fn write_image(
        &mut self,
        image: &DynamicImage,
        interpolate: bool,
        id: Option<Id>,
        transform: Option<Affine>,
    ) {
        let scaling = if interpolate { "smooth" } else { "pixelated" };

        let base64 = convert_image_to_base64_url(image);

        self.xml.start_element("image");
        if let Some(id) = id {
            self.xml.write_attribute("id", &id);
        }
        self.write_transform(transform);
        self.xml.write_attribute("xlink:href", &base64);
        self.xml.write_attribute("width", &image.width());
        self.xml.write_attribute("height", &image.height());
        self.xml.write_attribute("preserveAspectRatio", "none");
        self.xml
            .write_attribute("style", &format_args!("image-rendering: {scaling}"));
        self.xml.end_element();
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
            self.xml.write_attribute("d", &glyph.to_svg_f32());
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

    fn write_shading_pattern_defs(&mut self) {
        if self.shading_patterns.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "shading-pattern");

        for (id, shading) in self.shading_patterns.iter() {
            self.xml.start_element("pattern");
            self.xml.write_attribute("id", &id);
            self.xml.write_attribute("patternUnits", "userSpaceOnUse");
            self.xml.write_attribute("width", &shading.bbox.x1);
            self.xml.write_attribute("height", &shading.bbox.y1);
            self.xml.write_attribute(
                "patternTransform",
                &format!("matrix({})", convert_transform(&shading.transform)),
            );

            self.xml.start_element("use");
            self.xml
                .write_attribute("xlink:href", &format!("#{}", shading.shading));
            self.xml.end_element();

            self.xml.end_element();
        }

        self.xml.end_element();
    }

    fn write_tiling_pattern_defs(&mut self) {
        if self.tiling_patterns.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "tiling-pattern");

        let patterns = self
            .tiling_patterns
            .iter()
            .map(|i| (i.0, i.1.clone()))
            .collect::<Vec<_>>();

        for (id, pattern) in patterns {
            let pattern = pattern.clone();
            let transform = pattern.transform * pattern.tiling_pattern.matrix;

            self.xml.start_element("pattern");
            self.xml.write_attribute("id", &id);
            self.xml.write_attribute("patternUnits", "userSpaceOnUse");
            self.xml
                .write_attribute("width", &pattern.tiling_pattern.x_step);
            self.xml
                .write_attribute("height", &pattern.tiling_pattern.y_step);
            self.xml.write_attribute(
                "patternTransform",
                &format!("matrix({})", convert_transform(&transform)),
            );

            // TODO: Write bbox
            pattern
                .tiling_pattern
                .interpret(self, Affine::IDENTITY, false);

            self.xml.end_element();
        }

        self.xml.end_element();
    }

    fn write_shading_defs(&mut self) {
        if self.shadings.is_empty() {
            return;
        }

        let shadings = std::mem::take(&mut self.shadings);

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "shading");

        for (id, shading) in shadings.iter() {
            let encoded = shading.pattern.encode();
            let (image, transform) = render_texture(shading.bbox, &encoded);
            self.write_image(&image, true, Some(id), Some(transform));
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
    fn stroke_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        stroke_props: &StrokeProps,
    ) {
        self.transform = transform;
        self.stroke_props = stroke_props.clone();
        Self::stroke_path(self, path, paint);
    }

    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}

    fn fill_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        fill_rule: FillRule,
    ) {
        self.transform = transform;
        self.fill_rule = fill_rule;
        Self::fill_path(self, path, paint);
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        let clip_id = self.insert_clip(clip_path);

        self.xml.start_element("g");
        self.xml
            .write_attribute_fmt("clip-path", format_args!("url(#{clip_id})"));
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>) {}

    fn fill_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, paint: &Paint<'a>) {
        self.transform = transform;

        match glyph {
            Glyph::Outline(o) => {
                let id = self
                    .glyphs
                    .insert_with(o.identifier().cache_key(), || o.outline());

                match &paint {
                    Paint::Color(c) => {
                        self.xml.start_element("use");
                        self.xml
                            .write_attribute_fmt("xlink:href", format_args!("#{id}"));
                        self.write_transform(Some(self.transform * o.glyph_transform));

                        self.write_color(c, false);
                        self.xml.end_element();
                    }
                    Paint::Pattern(p) => match p.as_ref() {
                        Pattern::Shading(_) => {}
                        Pattern::Tiling(_) => {
                            unimplemented!()
                        }
                    },
                }
            }
            Glyph::Type3(_) => {}
        }
    }

    fn stroke_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        paint: &Paint,
        stroke_props: &StrokeProps,
    ) {
        self.stroke_props = stroke_props.clone();
        self.transform = transform;

        match glyph {
            Glyph::Outline(o) => {
                let path = o.glyph_transform * o.outline();
                let paint = paint.clone();
                self.stroke_path(&path, &paint);
            }
            Glyph::Type3(_) => {}
        }
    }

    fn draw_rgba_image(&mut self, image: RgbData, transform: Affine, alpha: Option<LumaData>) {
        self.transform = transform;

        let interpolate = image.interpolate;

        let image = if let Some(alpha) = alpha {
            if alpha.interpolate == image.interpolate
                && alpha.width == image.width
                && alpha.height == image.height
            {
                let interleaved = image
                    .data
                    .chunks(3)
                    .zip(alpha.data)
                    .flat_map(|(rgb, a)| [rgb[0], rgb[1], rgb[2], a])
                    .collect::<Vec<u8>>();

                DynamicImage::ImageRgba8(
                    ImageBuffer::from_raw(image.width, image.height, interleaved).unwrap(),
                )
            } else {
                unimplemented!();
            }
        } else {
            DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(image.width, image.height, image.data.clone()).unwrap(),
            )
        };

        self.write_image(&image, interpolate, None, None);
    }

    fn draw_stencil_image(&mut self, stencil: LumaData, transform: Affine, paint: &Paint) {
        self.transform = transform;

        let interpolate = stencil.interpolate;

        let image = match &paint {
            Paint::Color(c) => {
                let color = c.to_rgba().to_rgba8();
                let image = stencil
                    .data
                    .iter()
                    .flat_map(|d| if *d == 255 { color } else { [0, 0, 0, 0] })
                    .collect::<Vec<u8>>();

                DynamicImage::ImageRgba8(
                    ImageBuffer::from_raw(stencil.width, stencil.height, image).unwrap(),
                )
            }
            Paint::Pattern(_) => {
                unreachable!();
            }
        };

        self.write_image(&image, interpolate, None, None);
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
            transform: Affine::IDENTITY,
            fill_rule: FillRule::NonZero,
            stroke_props: StrokeProps::default(),
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

fn convert_transform(transform: &Affine) -> String {
    transform
        .as_coeffs()
        .iter()
        .map(|c| (*c as f32).to_string())
        .collect::<Vec<String>>()
        .join(" ")
}

fn convert_color(color: &Color) -> (String, f32) {
    let rgba8 = color.to_rgba().to_rgba8();
    let color = format!(
        "#{}",
        &rgba8[0..3]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let alpha = rgba8[3] as f32 / 255.0;

    (color, alpha)
}

pub fn convert_image_to_base64_url(image: &DynamicImage) -> String {
    let mut png_buffer = Vec::new();
    let mut cursor = Cursor::new(&mut png_buffer);
    image.write_to(&mut cursor, ImageFormat::Png).unwrap();

    let mut url = "data:image/png;base64,".to_string();
    let data = base64::engine::general_purpose::STANDARD.encode(png_buffer);
    url.push_str(&data);

    url
}

#[derive(Debug, Clone)]
struct Deduplicator<T> {
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

    fn insert_with<F>(&mut self, hash: u128, f: F) -> Id
    where
        F: FnOnce() -> T,
    {
        *self.present.entry(hash).or_insert_with(|| {
            let index = self.vec.len();
            self.vec.push(f());
            Id(self.kind, index as u64)
        })
    }

    fn iter(&self) -> impl Iterator<Item = (Id, &T)> {
        self.vec
            .iter()
            .enumerate()
            .map(|(i, v)| (Id(self.kind, i as u64), v))
    }

    fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Id(char, u64);

impl Display for Id {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.0, self.1)
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

fn render_texture(bbox: Rect, shading_pattern: &EncodedShadingPattern) -> (DynamicImage, Affine) {
    const SCALE: f32 = 2.0;
    const INV_SCALE: f32 = 1.0 / SCALE;

    let base_width = bbox.width() as f32;
    let base_height = bbox.height() as f32;

    let width = (base_width * SCALE).ceil() as u32;
    let height = (base_height * SCALE).ceil() as u32;

    let initial_transform = Affine::scale(INV_SCALE as f64)
        * shading_pattern.base_transform
        * Affine::translate((0.5, 0.5));
    let (x_advance, y_advance) = x_y_advances(&initial_transform);

    let mut buf = vec![0u8; width as usize * height as usize * 4];
    let mut start_point = initial_transform * Point::new(bbox.x0, bbox.y0);

    for row in buf.chunks_exact_mut(width as usize * 4) {
        let mut point = start_point;

        for pixel in row.chunks_exact_mut(4) {
            // println!("sampling {:?}", point);
            let sample = shading_pattern.sample(point);
            let converted = [
                (sample[0] * 255.0 + 0.5) as u8,
                (sample[1] * 255.0 + 0.5) as u8,
                (sample[2] * 255.0 + 0.5) as u8,
                (sample[3] * 255.0 + 0.5) as u8,
            ];

            pixel.copy_from_slice(&converted);

            point += x_advance;
        }

        start_point += y_advance;
    }

    let image = DynamicImage::ImageRgba8(ImageBuffer::from_raw(width, height, buf).unwrap());

    (
        image,
        Affine::translate((bbox.x0, bbox.y0)) * Affine::scale(INV_SCALE as f64),
    )
}

fn x_y_advances(transform: &Affine) -> (Vec2, Vec2) {
    let scale_skew_transform = {
        let c = transform.as_coeffs();
        Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
    };

    let x_advance = scale_skew_transform * Point::new(1.0, 0.0);
    let y_advance = scale_skew_transform * Point::new(0.0, 1.0);

    (
        Vec2::new(x_advance.x, x_advance.y),
        Vec2::new(y_advance.x, y_advance.y),
    )
}
