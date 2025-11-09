//! A number of utility methods.

use hayro_syntax::page::{Page, Rotation};
use kurbo::{Affine, Rect};
use log::warn;
use siphasher::sip128::{Hasher128, SipHasher13};
use skrifa::GlyphId;
use skrifa::raw::tables::cmap::CmapSubtable;
use std::hash::Hash;
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
                warn!("unsupported cmap table");
                None
            }
        }
    }
}

const SCALAR_NEARLY_ZERO: f32 = 1.0 / (1 << 8) as f32;

/// A number of useful methods for f32 numbers.
pub trait Float32Ext: Sized + Sub<f32, Output = f32> + Copy + PartialOrd<f32> {
    /// Whether the number is approximately 0.
    fn is_nearly_zero(&self) -> bool {
        self.is_nearly_zero_within_tolerance(SCALAR_NEARLY_ZERO)
    }

    /// Whether the number is nearly equal to another number.
    fn is_nearly_equal(&self, other: f32) -> bool {
        (*self - other).is_nearly_zero()
    }

    /// Whether the number is nearly equal to another number.
    fn is_nearly_less_or_equal(&self, other: f32) -> bool {
        (*self - other).is_nearly_zero() || *self < other
    }

    /// Whether the number is nearly equal to another number.
    fn is_nearly_greater_or_equal(&self, other: f32) -> bool {
        (*self - other).is_nearly_zero() || *self > other
    }

    /// Whether the number is approximately 0, with a given tolerance.
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool;
}

impl Float32Ext for f32 {
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool {
        debug_assert!(tolerance >= 0.0, "tolerance must be positive");

        self.abs() <= tolerance
    }
}

/// A number of useful methods for f64 numbers.
pub trait Float64Ext: Sized + Sub<f64, Output = f64> + Copy + PartialOrd<f64> {
    /// Whether the number is approximately 0.
    fn is_nearly_zero(&self) -> bool {
        self.is_nearly_zero_within_tolerance(SCALAR_NEARLY_ZERO as f64)
    }

    /// Whether the number is nearly equal to another number.
    fn is_nearly_equal(&self, other: f64) -> bool {
        (*self - other).is_nearly_zero()
    }

    /// Whether the number is nearly equal to another number.
    fn is_nearly_less_or_equal(&self, other: f64) -> bool {
        (*self - other).is_nearly_zero() || *self < other
    }

    /// Whether the number is nearly equal to another number.
    fn is_nearly_greater_or_equal(&self, other: f64) -> bool {
        (*self - other).is_nearly_zero() || *self > other
    }

    /// Whether the number is approximately 0, with a given tolerance.
    fn is_nearly_zero_within_tolerance(&self, tolerance: f64) -> bool;
}

impl Float64Ext for f64 {
    fn is_nearly_zero_within_tolerance(&self, tolerance: f64) -> bool {
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

/// Calculate a 128-bit siphash of a value.
pub(crate) fn hash128<T: Hash + ?Sized>(value: &T) -> u128 {
    let mut state = SipHasher13::new();
    value.hash(&mut state);
    state.finish128().as_u128()
}

/// Extension methods for rectangles.
pub trait RectExt {
    /// Convert the rectangle to a `kurbo` rectangle.
    fn to_kurbo(&self) -> kurbo::Rect;
}

impl RectExt for hayro_syntax::object::Rect {
    fn to_kurbo(&self) -> Rect {
        Rect::new(self.x0, self.y0, self.x1, self.y1)
    }
}

// Note: Keep in sync with `hayro-write`.
/// Extension methods for PDF pages.
pub trait PageExt {
    /// Return the initial transform that should be applied when rendering. This accounts for a
    /// number of factors, such as the mismatch between PDF's y-up and most renderers' y-down
    /// coordinate system, the rotation of the page and the offset of the crop box.
    fn initial_transform(&self, invert_y: bool) -> kurbo::Affine;
}

impl PageExt for Page<'_> {
    fn initial_transform(&self, invert_y: bool) -> kurbo::Affine {
        let crop_box = self.intersected_crop_box();
        let (_, base_height) = self.base_dimensions();
        let (width, height) = self.render_dimensions();

        let horizontal_t =
            Affine::rotate(90.0f64.to_radians()) * Affine::translate((0.0, -width as f64));
        let flipped_horizontal_t =
            Affine::translate((0.0, height as f64)) * Affine::rotate(-90.0f64.to_radians());

        let rotation_transform = match self.rotation() {
            Rotation::None => Affine::IDENTITY,
            Rotation::Horizontal => {
                if invert_y {
                    horizontal_t
                } else {
                    flipped_horizontal_t
                }
            }
            Rotation::Flipped => {
                Affine::scale(-1.0) * Affine::translate((-width as f64, -height as f64))
            }
            Rotation::FlippedHorizontal => {
                if invert_y {
                    flipped_horizontal_t
                } else {
                    horizontal_t
                }
            }
        };

        let inversion_transform = if invert_y {
            Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, base_height as f64])
        } else {
            Affine::IDENTITY
        };

        rotation_transform * inversion_transform * Affine::translate((-crop_box.x0, -crop_box.y0))
    }
}
