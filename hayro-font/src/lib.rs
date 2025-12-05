/*!
A CFF and Type1 font parser.

This crate is a fork of the [`ttf-parser`](https://github.com/harfbuzz/ttf-parser) library,
but with the majority of the functionality completely stripped away.
The purpose of this crate is to be a light-weight font parser for CFF and Type1 fonts,
as they can be found in PDFs. Only the code for parsing CFF fonts has been retained, while code
for parsing Type1 fonts was newly added.

Note that this is an internal crate and not meant to be used directly. Therefore,
it's not well-documented.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use crate::util::TryNumFrom;

pub mod cff;
pub mod type1;

mod argstack;
mod util;

/// A type-safe wrapper for glyph ID.
#[repr(transparent)]
#[derive(Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Default, Debug, Hash)]
pub struct GlyphId(pub u16);

/// A trait for glyph outline construction.
pub trait OutlineBuilder {
    /// Appends a `MoveTo` segment.
    ///
    /// Start of a contour.
    fn move_to(&mut self, x: f32, y: f32);

    /// Appends a `LineTo` segment.
    fn line_to(&mut self, x: f32, y: f32);

    /// Appends a `QuadTo` segment.
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32);

    /// Appends a `CurveTo` segment.
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32);

    /// Appends a `ClosePath` segment.
    ///
    /// End of a contour.
    fn close(&mut self);
}

struct DummyOutline;
impl OutlineBuilder for DummyOutline {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

/// A rectangle.
///
/// Doesn't guarantee that `x_min` <= `x_max` and/or `y_min` <= `y_max`.
#[repr(C)]
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
}

impl Rect {
    #[inline]
    fn zero() -> Self {
        Self {
            x_min: 0,
            y_min: 0,
            x_max: 0,
            y_max: 0,
        }
    }

    /// Returns rect's width.
    #[inline]
    pub fn width(&self) -> i16 {
        self.x_max - self.x_min
    }

    /// Returns rect's height.
    #[inline]
    pub fn height(&self) -> i16 {
        self.y_max - self.y_min
    }
}

/// A rectangle described by the left-lower and upper-right points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    /// The horizontal minimum of the rect.
    pub x_min: f32,
    /// The vertical minimum of the rect.
    pub y_min: f32,
    /// The horizontal maximum of the rect.
    pub x_max: f32,
    /// The vertical maximum of the rect.
    pub y_max: f32,
}

impl RectF {
    #[inline]
    fn new() -> Self {
        Self {
            x_min: f32::MAX,
            y_min: f32::MAX,
            x_max: f32::MIN,
            y_max: f32::MIN,
        }
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.x_min == f32::MAX
            && self.y_min == f32::MAX
            && self.x_max == f32::MIN
            && self.y_max == f32::MIN
    }

    #[inline]
    fn extend_by(&mut self, x: f32, y: f32) {
        self.x_min = self.x_min.min(x);
        self.y_min = self.y_min.min(y);
        self.x_max = self.x_max.max(x);
        self.y_max = self.y_max.max(y);
    }

    #[inline]
    fn to_rect(self) -> Option<Rect> {
        Some(Rect {
            x_min: i16::try_num_from(self.x_min)?,
            y_min: i16::try_num_from(self.y_min)?,
            x_max: i16::try_num_from(self.x_max)?,
            y_max: i16::try_num_from(self.y_max)?,
        })
    }
}

pub(crate) struct Builder<'a> {
    pub(crate) builder: &'a mut dyn OutlineBuilder,
    pub(crate) bbox: RectF,
}

impl<'a> Builder<'a> {
    #[inline]
    fn move_to(&mut self, x: f32, y: f32) {
        self.bbox.extend_by(x, y);
        self.builder.move_to(x, y);
    }

    #[inline]
    fn line_to(&mut self, x: f32, y: f32) {
        self.bbox.extend_by(x, y);
        self.builder.line_to(x, y);
    }

    #[inline]
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.bbox.extend_by(x1, y1);
        self.bbox.extend_by(x2, y2);
        self.bbox.extend_by(x, y);
        self.builder.curve_to(x1, y1, x2, y2, x, y);
    }

    #[inline]
    fn close(&mut self) {
        self.builder.close();
    }
}

/// An affine transformation matrix.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug)]
pub struct Matrix {
    pub sx: f32,
    pub ky: f32,
    pub kx: f32,
    pub sy: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Matrix {
    fn default() -> Self {
        Self {
            sx: 0.001,
            ky: 0.0,
            kx: 0.0,
            sy: 0.001,
            tx: 0.0,
            ty: 0.0,
        }
    }
}

/// A list of errors that can occur during CFF/Type1 glyph outlining.
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutlineError {
    NoGlyph,
    ReadOutOfBounds,
    ZeroBBox,
    InvalidOperator,
    UnsupportedOperator,
    MissingEndChar,
    DataAfterEndChar,
    NestingLimitReached,
    ArgumentsStackLimitReached,
    InvalidArgumentsStackLength,
    BboxOverflow,
    MissingMoveTo,
    InvalidSubroutineIndex,
    NoLocalSubroutines,
    InvalidSeacCode,
}
