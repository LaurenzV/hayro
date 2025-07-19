//! A number of utility methods.

use log::warn;
use skrifa::GlyphId;
use skrifa::raw::tables::cmap::CmapSubtable;
use std::ops::Sub;

pub(crate) trait OptionLog {
    fn warn_none(self, f: &str) -> Self;
}

impl<T> OptionLog for Option<T> {
    #[inline]
    fn warn_none(self, f: &str) -> Self {
        self.or_else(|| {
            warn!("{f}");

            None
        })
    }
}

pub(crate) trait CodeMapExt {
    fn map_codepoint(&self, code: impl Into<u32>) -> Option<GlyphId>;
}

impl CodeMapExt for CmapSubtable<'_> {
    fn map_codepoint(&self, code: impl Into<u32>) -> Option<GlyphId> {
        match self {
            CmapSubtable::Format0(f) => f.map_codepoint(code),
            CmapSubtable::Format4(f) => f.map_codepoint(code),
            CmapSubtable::Format6(f) => f.map_codepoint(code),
            CmapSubtable::Format12(f) => f.map_codepoint(code),
            _ => {
                warn!("unsupported cmap table {self:?}");

                None
            }
        }
    }
}

const SCALAR_NEARLY_ZERO: f32 = 1.0 / (1 << 12) as f32;

/// A number of useful methods for f32 numbers.
pub trait FloatExt: Sized + Sub<f32, Output = f32> + Copy {
    /// Whether the number is approximately 0.
    fn is_nearly_zero(&self) -> bool {
        self.is_nearly_zero_within_tolerance(SCALAR_NEARLY_ZERO)
    }

    /// Whether the number is nearly equal to another number.
    fn is_nearly_equal(&self, other: f32) -> bool {
        (*self - other).is_nearly_zero()
    }

    /// Whether the number is approximately 0, with a given tolerance.
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool;
}

impl FloatExt for f32 {
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool {
        debug_assert!(tolerance >= 0.0, "tolerance must be positive");

        self.abs() <= tolerance
    }
}

pub(crate) trait PointExt: Sized {
    fn x(&self) -> f32;
    fn y(&self) -> f32;

    fn nearly_same(&self, other: Self) -> bool {
        self.x().is_nearly_equal(other.x()) && self.y().is_nearly_equal(other.y())
    }
}

impl PointExt for kurbo::Point {
    fn x(&self) -> f32 {
        self.x as f32
    }

    fn y(&self) -> f32 {
        self.y as f32
    }
}
