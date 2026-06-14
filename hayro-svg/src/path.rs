use crate::SvgRenderer;
use hayro_interpret::{DrawMode, DrawProps, FillRule};
use kurbo::{BezPath, PathEl, Rect, Shape};
use std::io;
use std::io::Write;

impl<'a> SvgRenderer<'a> {
    pub(crate) fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        let svg_path = path.to_svg_f32();

        self.xml.start_element("path");
        self.xml.write_attribute("d", &svg_path);

        match draw_mode {
            DrawMode::Fill(f) => {
                if *f == FillRule::EvenOdd {
                    self.xml.write_attribute("fill-rule", "evenodd");
                }
                self.write_paint(&props.paint, || path.bounding_box(), props.transform, None);
            }
            DrawMode::Stroke(s) => {
                self.write_stroke_properties(s);
                self.write_paint(
                    &props.paint,
                    || path.bounding_box(),
                    props.transform,
                    Some(s),
                );
            }
            DrawMode::FillAndStroke(f, s) => {
                if *f == FillRule::EvenOdd {
                    self.xml.write_attribute("fill-rule", "evenodd");
                }
                self.write_stroke_properties(s);
                self.write_fill_and_stroke_paint(
                    &props.paint,
                    || path.bounding_box(),
                    props.transform,
                    s,
                );
            }
            DrawMode::Invisible => {
                self.xml.end_element();
                return;
            }
        }

        self.write_transform(props.transform);
        self.xml.end_element();
    }

    pub(crate) fn draw_rect(&mut self, rect: &Rect, props: DrawProps<'a>, draw_mode: &DrawMode) {
        self.xml.start_element("rect");
        self.xml.write_attribute("x", &rect.x0);
        self.xml.write_attribute("y", &rect.y0);
        self.xml.write_attribute("width", &rect.width());
        self.xml.write_attribute("height", &rect.height());

        match draw_mode {
            DrawMode::Fill(_) => {
                self.write_paint(&props.paint, || *rect, props.transform, None);
            }
            DrawMode::Stroke(s) => {
                self.write_stroke_properties(s);
                self.write_paint(&props.paint, || *rect, props.transform, Some(s));
            }
            DrawMode::FillAndStroke(_, s) => {
                self.write_stroke_properties(s);
                self.write_fill_and_stroke_paint(&props.paint, || *rect, props.transform, s);
            }
            DrawMode::Invisible => {
                self.xml.end_element();
                return;
            }
        }

        self.write_transform(props.transform);
        self.xml.end_element();
    }
}

pub(crate) trait BezPathExt {
    fn to_svg_f32(&self) -> String {
        let mut buffer = Vec::new();
        self.write_to_f32(&mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    fn write_to_f32<W: Write>(&self, writer: W) -> io::Result<()>;
}

impl BezPathExt for BezPath {
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
