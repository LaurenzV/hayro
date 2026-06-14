use crate::SvgRenderer;
use crate::path::BezPathExt;
use hayro_interpret::FillRule;
use kurbo::{BezPath, Rect};

pub(crate) enum CachedClipPath {
    Path { path: BezPath, fill_rule: FillRule },
    Rect(Rect),
}

impl SvgRenderer<'_> {
    pub(crate) fn write_clip_path_defs(&mut self) {
        if self.clip_paths.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "clip-path");

        for (id, clip_path) in self.clip_paths.iter() {
            self.xml.start_element("clipPath");
            self.xml.write_attribute("id", &id);

            match clip_path {
                CachedClipPath::Path { path, fill_rule } => {
                    self.xml.start_element("path");
                    self.xml.write_attribute("d", &path.to_svg_f32());

                    if *fill_rule == FillRule::EvenOdd {
                        self.xml.write_attribute("clip-rule", "evenodd");
                    }

                    self.xml.end_element();
                }
                CachedClipPath::Rect(rect) => {
                    self.xml.start_element("rect");
                    self.xml.write_attribute("x", &rect.x0);
                    self.xml.write_attribute("y", &rect.y0);
                    self.xml.write_attribute("width", &rect.width());
                    self.xml.write_attribute("height", &rect.height());
                    self.xml.end_element();
                }
            }

            self.xml.end_element();
        }

        self.xml.end_element();
    }
}
