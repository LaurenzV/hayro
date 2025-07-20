use hayro_interpret::font::Glyph;
use hayro_interpret::{
    ClipPath, Device, FillProps, LumaData, Paint, PaintType, RgbData, SoftMask, StrokeProps,
};
use kurbo::{Affine, BezPath};
use xmlwriter::{Options, XmlWriter};

pub(crate) struct SvgRenderer {
    xml: XmlWriter,
    transform: Affine,
    fill_props: FillProps,
    stroke_props: StrokeProps,
}

impl SvgRenderer {
    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        let svg_path = path.to_svg();
        let (fill, alpha) = convert_paint(paint);

        self.xml.start_element("path");
        self.xml.write_attribute("d", &svg_path);
        self.xml.write_attribute("fill", &fill);
        self.xml.write_attribute("fill-opacity", &alpha);
        self.write_transform();
        self.xml.end_element();
    }

    fn stroke_path(&mut self, path: &BezPath, paint: &Paint) {
        let svg_path = path.to_svg();
        let (fill, alpha) = convert_paint(paint);

        self.xml.start_element("path");
        self.xml.write_attribute("d", &svg_path);
        self.xml.write_attribute("stroke", &fill);
        self.xml.write_attribute("stroke-opacity", &alpha);
        self.xml.write_attribute("fill", "none");
        self.write_transform();
        self.xml.end_element();
    }

    fn write_transform(&mut self) {
        self.xml.write_attribute(
            "transform",
            &format!("matrix({})", &convert_transform(&self.transform)),
        );
    }
}

impl Device for SvgRenderer {
    fn set_transform(&mut self, affine: Affine) {
        self.transform = affine;
    }

    fn stroke_path(&mut self, path: &BezPath, paint: &Paint) {
        Self::stroke_path(self, path, paint);
    }

    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps) {
        self.stroke_props = stroke_props.clone();
    }

    fn set_soft_mask(&mut self, mask: Option<SoftMask>) {}

    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        Self::fill_path(self, path, paint);
    }

    fn set_fill_properties(&mut self, fill_props: &FillProps) {
        self.fill_props = fill_props.clone();
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {}

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask>) {}

    fn fill_glyph(&mut self, glyph: &Glyph<'_>, paint: &Paint) {
        match glyph {
            Glyph::Outline(o) => {
                let path = o.glyph_transform * o.outline();
                let paint = paint.clone();
                self.fill_path(&path, &paint);
            }
            Glyph::Type3(_) => {}
        }
    }

    fn stroke_glyph(&mut self, glyph: &Glyph<'_>, paint: &Paint) {
        match glyph {
            Glyph::Outline(o) => {
                let path = o.glyph_transform * o.outline();
                let paint = paint.clone();
                self.stroke_path(&path, &paint);
            }
            Glyph::Type3(_) => {}
        }
    }

    fn draw_rgba_image(&mut self, image: RgbData, alpha: Option<LumaData>) {}

    fn draw_stencil_image(&mut self, stencil: LumaData, paint: &Paint) {}

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}

impl SvgRenderer {
    pub(crate) fn new() -> Self {
        Self {
            xml: XmlWriter::new(Options::default()),
            transform: Affine::IDENTITY,
            fill_props: FillProps::default(),
            stroke_props: StrokeProps::default(),
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
        self.xml.end_element();

        self.xml.end_document()
    }
}

fn convert_transform(transform: &Affine) -> String {
    transform
        .as_coeffs()
        .iter()
        .map(|c| c.to_string())
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
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>()
            );
            let alpha = rgba8[3] as f32 / 255.0;

            (color, alpha)
        }
        PaintType::Pattern(_) => ("black".to_string(), 1.0),
    }
}
