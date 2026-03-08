// Copyright 2018 the Kurbo Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Affine transforms.

use core::ops::Mul;

/// A 2D affine transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine([f64; 6]);

impl Affine {
    /// The identity transform.
    pub const IDENTITY: Affine = Affine::scale(1.0);

    /// Construct an affine transform from coefficients.
    ///
    /// If the coefficients are `(a, b, c, d, e, f)`, then the resulting
    /// transformation represents this augmented matrix:
    ///
    /// ```text
    /// | a c e |
    /// | b d f |
    /// | 0 0 1 |
    /// ```
    ///
    /// Note that this convention is transposed from PostScript and
    /// Direct2D, but is consistent with the
    /// [Wikipedia](https://en.wikipedia.org/wiki/Affine_transformation)
    /// formulation of affine transformation as augmented matrix. The
    /// idea is that `(A * B) * v == A * (B * v)`, where `*` is the
    /// [`Mul`] trait.
    #[inline(always)]
    pub const fn new(c: [f64; 6]) -> Affine {
        Affine(c)
    }

    /// An affine transform representing uniform scaling.
    #[inline(always)]
    pub(crate) const fn scale(s: f64) -> Affine {
        Affine([s, 0.0, 0.0, s, 0.0, 0.0])
    }

    /// A clockwise 90° rotation (in a Y-down coordinate system).
    pub(crate) const ROTATE_CW_90: Affine = Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);

    /// A counter-clockwise 90° rotation (in a Y-down coordinate system).
    pub(crate) const ROTATE_CCW_90: Affine = Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]);

    /// An affine transform representing translation.
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
