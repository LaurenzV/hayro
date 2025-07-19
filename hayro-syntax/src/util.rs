use log::error;
use std::ops::Sub;

pub(crate) trait OptionLog {
    fn error_none(self, f: &str) -> Self;
}

impl<T> OptionLog for Option<T> {
    fn error_none(self, f: &str) -> Self {
        self.or_else(|| {
            error!("{f}");

            None
        })
    }
}

const SCALAR_NEARLY_ZERO: f32 = 1.0 / (1 << 12) as f32;

/// A number of useful methods for f32 numbers.
pub trait FloatExt: Sized + Sub<f32, Output = f32> + Copy {
    /// Whether the number is approximately 0.
    fn is_nearly_zero(&self) -> bool {
        self.is_nearly_zero_within_tolerance(SCALAR_NEARLY_ZERO)
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
