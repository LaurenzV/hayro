use base64::Engine;
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    CacheKey, ClipPath, Device, FillRule, LumaData, Paint, PaintType, RgbData, SoftMask,
    StrokeProps,
};
use image::{DynamicImage, ImageBuffer, ImageFormat};
use kurbo::{Affine, BezPath, PathEl};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::io::{Cursor, Write};
use std::{fmt, io};
use xmlwriter::{Options, XmlWriter};

struct CachedClipPath {
    path: BezPath,
    fill_rule: FillRule,
}

pub(crate) struct SvgRenderer {
    xml: XmlWriter,
    transform: Affine,
    fill_rule: FillRule,
    stroke_props: StrokeProps,
    glyphs: Deduplicator<BezPath>,
    clip_paths: Deduplicator<CachedClipPath>,
}

impl SvgRenderer {
    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        let svg_path = path.to_svg_f32();

        self.xml.start_element("path");
        self.xml.write_attribute("d", &svg_path);
        self.write_paint(paint, false);
        self.write_transform(None);
        self.xml.end_element();
    }

    fn write_paint(&mut self, paint: &Paint, is_stroke: bool) {
        let (fill, alpha) = convert_paint(paint);

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

        self.xml.start_element("path");
        self.xml.write_attribute("d", &svg_path);
        self.write_paint(paint, true);
        self.xml.write_attribute("fill", "none");
        self.write_transform(None);
        self.xml.end_element();
    }

    fn write_transform(&mut self, transform: Option<Affine>) {
        self.xml.write_attribute(
            "transform",
            &format!(
                "matrix({})",
                &convert_transform(&transform.unwrap_or(self.transform))
            ),
        );
    }

    fn write_image(&mut self, image: &DynamicImage, interpolate: bool) {
        let scaling = if interpolate { "smooth" } else { "pixelated" };

        let base64 = convert_image_to_base64_url(image);

        self.xml.start_element("image");
        self.xml.write_attribute("xlink:href", &base64);
        self.write_transform(None);
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
}

impl Device for SvgRenderer {
    fn stroke_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint,
        stroke_props: &StrokeProps,
    ) {
        self.transform = transform;
        self.stroke_props = stroke_props.clone();
        Self::stroke_path(self, path, paint);
    }

    fn set_soft_mask(&mut self, _: Option<SoftMask>) {}

    fn fill_path(&mut self, path: &BezPath, transform: Affine, paint: &Paint, fill_rule: FillRule) {
        self.transform = transform;
        self.fill_rule = fill_rule;
        Self::fill_path(self, path, paint);
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

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask>) {}

    fn fill_glyph(&mut self, glyph: &Glyph<'_>, transform: Affine, paint: &Paint) {
        self.transform = transform;

        match glyph {
            Glyph::Outline(o) => {
                let id = self
                    .glyphs
                    .insert_with(o.identifier().cache_key(), || o.outline());

                self.xml.start_element("use");
                self.xml
                    .write_attribute_fmt("xlink:href", format_args!("#{id}"));
                self.write_transform(Some(self.transform * o.glyph_transform));
                self.write_paint(paint, false);
                self.xml.end_element();
            }
            Glyph::Type3(_) => {}
        }
    }

    fn stroke_glyph(
        &mut self,
        glyph: &Glyph<'_>,
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

        self.write_image(&image, interpolate);
    }

    fn draw_stencil_image(&mut self, stencil: LumaData, transform: Affine, paint: &Paint) {
        self.transform = transform;

        let interpolate = stencil.interpolate;

        let image = match &paint.paint_type {
            PaintType::Color(c) => {
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
            PaintType::Pattern(_) => {
                unreachable!();
            }
        };

        self.write_image(&image, interpolate);
    }

    fn pop_clip_path(&mut self) {
        self.xml.end_element();
    }

    fn pop_transparency_group(&mut self) {}
}

impl SvgRenderer {
    pub(crate) fn new() -> Self {
        Self {
            xml: XmlWriter::new(Options::default()),
            transform: Affine::IDENTITY,
            fill_rule: FillRule::NonZero,
            stroke_props: StrokeProps::default(),
            glyphs: Deduplicator::new('g'),
            clip_paths: Deduplicator::new('c'),
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
        self.write_glyph_defs();
        self.write_clip_path_defs();
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

fn convert_paint(paint: &Paint) -> (String, f32) {
    match &paint.paint_type {
        PaintType::Color(c) => {
            let rgba8 = c.to_rgba().to_rgba8();
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
        PaintType::Pattern(_) => ("black".to_string(), 1.0),
    }
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
