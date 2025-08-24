use crate::SvgRenderer;
use crate::path::BezPathExt;
use hayro_interpret::FillRule;
use kurbo::BezPath;

pub(crate) struct CachedClipPath {
    pub(crate) path: BezPath,
    pub(crate) fill_rule: FillRule,
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
