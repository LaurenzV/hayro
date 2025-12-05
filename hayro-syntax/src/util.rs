use log::error;
use std::ops::Sub;
use std::sync::OnceLock;

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
pub(crate) trait FloatExt: Sized + Sub<f32, Output = f32> + Copy {
    /// Whether the number is approximately 0.
    fn is_nearly_zero(&self) -> bool {
        self.is_nearly_zero_within_tolerance(SCALAR_NEARLY_ZERO)
    }

    /// Whether the number is approximately 0, with a given tolerance.
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool;
}

impl FloatExt for f32 {
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool {
        debug_assert!(tolerance >= 0.0, "tolerance must be non-negative");

        self.abs() <= tolerance
    }
}

/// Allows to store elements at an index with an `&self` reference.
///
/// This is similar to a thread-safe arena data structure, but with less moving
/// parts and no unsafe code. It's based on the segment list presented in
/// <https://matklad.github.io/2023/04/23/data-oriented-parallel-value-interner.html>
///
/// Indices should be used in order. Usage of higher indices implies more memory
/// usage (even if lower indices are not in use).
///
/// The capacity is limited at 2^C - 1. Calling `get_or_init` with a higher
/// index will lead to a panic.
pub(crate) struct SegmentList<T, const C: usize>([OnceLock<Box<[OnceLock<T>]>>; C]);

impl<T, const C: usize> SegmentList<T, C> {
    pub(crate) fn new() -> Self {
        Self(std::array::from_fn(|_| OnceLock::new()))
    }

    pub(crate) fn get(&self, i: usize) -> Option<&T> {
        let (s, k) = self.locate(i);
        let segment = self.0[s as usize].get()?;
        segment.get(k)?.get()
    }

    #[track_caller]
    pub(crate) fn get_or_init(&self, i: usize, f: impl FnOnce() -> T) -> &T {
        let (s, k) = self.locate(i);
        let segment = self
            .0
            .get(s as usize)
            .expect("segment list is out of capacity")
            .get_or_init(|| {
                (0..2_usize.pow(s))
                    .map(|_| OnceLock::new())
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            });
        segment[k].get_or_init(f)
    }

    fn locate(&self, i: usize) -> (u32, usize) {
        let power = (i + 2).next_power_of_two() / 2;
        let s = power.trailing_zeros();
        let k = i - ((1 << s) - 1);
        (s, k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_list() {
        let e = SegmentList::<String, 10>::new();

        // Ensure that it works.
        for i in 0..500 {
            e.get_or_init(i, || format!("{i}"));
        }
        for i in 0..500 {
            assert_eq!(e.get(i), Some(&format!("{i}")));
        }

        // Ensure that all slots in the first 8 segments are actually in use,
        // i.e. we didn't overallocate.
        for s in 0..8 {
            assert!(e.0[s].get().unwrap().iter().all(|s| s.get().is_some()));
        }
    }
}
