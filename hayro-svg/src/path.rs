use crate::SvgRenderer;
use hayro_interpret::{Paint, PathDrawMode};
use kurbo::{Affine, BezPath, PathEl};
use std::io;
use std::io::Write;

impl<'a> SvgRenderer<'a> {
    pub(crate) fn draw_path(
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
