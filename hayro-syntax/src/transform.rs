// Copyright 2018 the Kurbo Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! 2D affine transformations.

use core::ops::Mul;

/// A 2D affine transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine([f64; 6]);

impl Affine {
    /// The identity transform.
    pub const IDENTITY: Affine = Affine::scale(1.0);

    #[inline(always)]
    pub(crate) const fn new(c: [f64; 6]) -> Affine {
        Affine(c)
    }

    #[inline(always)]
    pub(crate) const fn scale(s: f64) -> Affine {
        Affine([s, 0.0, 0.0, s, 0.0, 0.0])
    }

    pub(crate) const ROTATE_CW_90: Affine = Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);

    pub(crate) const ROTATE_CCW_90: Affine = Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]);

    #[inline(always)]
    pub(crate) const fn translate(p: (f64, f64)) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, p.0, p.1])
    }

    /// Get the coefficients of the transform.
    #[inline(always)]
    pub const fn as_coeffs(self) -> [f64; 6] {
        self.0
    }
}

impl Mul for Affine {
    type Output = Affine;

    #[inline]
    fn mul(self, other: Affine) -> Affine {
        Affine([
            self.0[0] * other.0[0] + self.0[2] * other.0[1],
            self.0[1] * other.0[0] + self.0[3] * other.0[1],
            self.0[0] * other.0[2] + self.0[2] * other.0[3],
            self.0[1] * other.0[2] + self.0[3] * other.0[3],
            self.0[0] * other.0[4] + self.0[2] * other.0[5] + self.0[4],
            self.0[1] * other.0[4] + self.0[3] * other.0[5] + self.0[5],
        ])
    }
}
