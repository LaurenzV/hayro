use crate::CacheKey;
use crate::font::cid::Type0Font;
use crate::font::true_type::TrueTypeFont;
use crate::font::type1::Type1Font;
use hayro_font::OutlineBuilder;
use kurbo::BezPath;
use skrifa::GlyphId;
use skrifa::outline::OutlinePen;
use std::rc::Rc;

pub(crate) struct OutlinePath(BezPath);

impl OutlinePath {
    pub(crate) fn new() -> Self {
        Self(BezPath::new())
    }

    pub(crate) fn take(self) -> BezPath {
        self.0
    }
}

impl OutlinePen for OutlinePath {
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

impl OutlineBuilder for OutlinePath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.line_to((x, y));
        }
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.quad_to((x1, y1), (x, y));
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        if !self.0.elements().is_empty() {
            self.0.curve_to((x1, y1), (x2, y2), (x, y));
        }
    }

    fn close(&mut self) {
        if !self.0.elements().is_empty() {
            self.0.close_path();
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum OutlineFont {
    Type1(Rc<Type1Font>),
    TrueType(Rc<TrueTypeFont>),
    Type0(Rc<Type0Font>),
}

impl CacheKey for OutlineFont {
    fn cache_key(&self) -> u128 {
        match self {
            Self::Type1(f) => f.cache_key(),
            Self::TrueType(t) => t.cache_key(),
            Self::Type0(t0) => t0.cache_key(),
        }
    }
}

impl OutlineFont {
    pub(crate) fn outline_glyph(&self, glyph: GlyphId) -> BezPath {
        match self {
            Self::Type1(t) => t.outline_glyph(glyph),
            Self::TrueType(t) => t.outline_glyph(glyph),
            Self::Type0(t) => t.outline_glyph(glyph),
        }
    }

    pub(crate) fn char_code_to_unicode(&self, char_code: u32) -> Option<char> {
        match self {
            Self::Type1(t) => t.char_code_to_unicode(char_code),
            Self::TrueType(t) => t.char_code_to_unicode(char_code),
            Self::Type0(t) => t.char_code_to_unicode(char_code),
        }
    }
}
