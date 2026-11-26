use crate::SvgRenderer;
use hayro_interpret::{DrawMode, DrawProps, FillRule, StrokeProps};
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
                self.write_paint(
                    &props.paint,
                    |stroke| path_paint_bounds(path, stroke),
                    props.transform,
                    None,
                );
            }
            DrawMode::Stroke(s) => {
                self.write_stroke_properties(s);
                self.write_paint(
                    &props.paint,
                    |stroke| path_paint_bounds(path, stroke),
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
                    |stroke| path_paint_bounds(path, stroke),
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
                self.write_paint(
                    &props.paint,
                    |stroke| rect_paint_bounds(*rect, stroke),
                    props.transform,
                    None,
                );
            }
            DrawMode::Stroke(s) => {
                self.write_stroke_properties(s);
                self.write_paint(
                    &props.paint,
                    |stroke| rect_paint_bounds(*rect, stroke),
                    props.transform,
                    Some(s),
                );
            }
            DrawMode::FillAndStroke(_, s) => {
                self.write_stroke_properties(s);
                self.write_fill_and_stroke_paint(
                    &props.paint,
                    |stroke| rect_paint_bounds(*rect, stroke),
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
}

fn rect_paint_bounds(rect: Rect, stroke: Option<&StrokeProps>) -> Rect {
    let pad = stroke.map_or(0., |s| f64::from(s.line_width));
    rect.inflate(pad, pad)
}

pub(crate) fn path_paint_bounds(path: &BezPath, stroke: Option<&StrokeProps>) -> Rect {
    let bounds = rect_paint_bounds(path.bounding_box(), stroke);
    let Some(props) = stroke else { return bounds };
    if props.line_join != kurbo::Join::Miter
        || props.miter_limit <= 2.
        || path.elements().len() <= 2
    {
        return bounds;
    }
    // Undashed joins cover all miter tips; the width padding already covers dash caps.
    let style = kurbo::Stroke::new(f64::from(props.line_width))
        .with_join(kurbo::Join::Miter)
        .with_miter_limit(f64::from(props.miter_limit));
    let tolerance = f64::from(props.line_width).abs().max(1.) / 256.;
    let joins = kurbo::stroke(path, &style, &kurbo::StrokeOpts::default(), tolerance);
    bounds.union(joins.bounding_box().inflate(tolerance, tolerance))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miter_limit_does_not_enlarge_a_single_segment() {
        let mut path = BezPath::new();
        path.move_to((0., 0.));
        path.line_to((8., 0.));
        let stroke = StrokeProps {
            line_width: 2.,
            miter_limit: 10000.,
            ..Default::default()
        };
        assert_eq!(
            path_paint_bounds(&path, Some(&stroke)),
            Rect::new(-2., -2., 10., 2.)
        );
    }
}
