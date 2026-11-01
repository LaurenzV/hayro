use kurbo::BezPath;
use skrifa::PathSink;

pub(crate) struct OutlinePath(BezPath);

impl OutlinePath {
    pub(crate) fn new() -> Self {
        Self(BezPath::new())
    }

    pub(crate) fn take(self) -> BezPath {
        self.0
    }
}

impl PathSink for OutlinePath {
    #[inline]
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x, y));
    }

    #[inline]
    fn line_to(&mut self, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.line_to((x, y));
        }
    }

    #[inline]
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.quad_to((cx, cy), (x, y));
        }
    }

    #[inline]
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.curve_to((cx0, cy0), (cx1, cy1), (x, y));
        }
    }

    #[inline]
    fn close(&mut self) {
        if !self.0.elements().is_empty() {
            self.0.close_path();
        }
    }
}
