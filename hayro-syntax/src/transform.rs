// Copyright 2018 the Kurbo Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Affine transforms.

use crate::object::Rect;
use core::ops::{Mul, MulAssign};

/// A 2D affine transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine([f64; 6]);

impl Affine {
    /// The identity transform.
    pub const IDENTITY: Affine = Affine::scale(1.0);

    /// A transform that is flipped on the y-axis. Useful for converting between
    /// y-up and y-down spaces.
    pub const FLIP_Y: Affine = Affine::new([1.0, 0., 0., -1.0, 0., 0.]);

    /// A transform that is flipped on the x-axis.
    pub const FLIP_X: Affine = Affine::new([-1.0, 0., 0., 1.0, 0., 0.]);

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
    pub const fn scale(s: f64) -> Affine {
        Affine([s, 0.0, 0.0, s, 0.0, 0.0])
    }

    /// An affine transform representing non-uniform scaling
    /// with different scale values for x and y.
    #[inline(always)]
    pub const fn scale_non_uniform(s_x: f64, s_y: f64) -> Affine {
        Affine([s_x, 0.0, 0.0, s_y, 0.0, 0.0])
    }

    /// An affine transform representing a scale of `scale` about `center`.
    ///
    /// Useful for a view transform that zooms at a specific point,
    /// while keeping that point fixed in the result space.
    #[inline]
    pub fn scale_about(s: f64, center: (f64, f64)) -> Affine {
        Self::translate((-center.0, -center.1))
            .then_scale(s)
            .then_translate(center)
    }

    /// An affine transform representing rotation.
    ///
    /// The convention for rotation is that a positive angle rotates a
    /// positive X direction into positive Y. Thus, in a Y-down coordinate
    /// system (as is common for graphics), it is a clockwise rotation, and
    /// in Y-up (traditional for math), it is anti-clockwise.
    ///
    /// The angle, `th`, is expressed in radians.
    #[inline]
    pub fn rotate(th: f64) -> Affine {
        let (s, c) = th.sin_cos();
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    /// An affine transform representing a rotation of `th` radians about `center`.
    #[inline]
    pub fn rotate_about(th: f64, center: (f64, f64)) -> Affine {
        Self::translate((-center.0, -center.1))
            .then_rotate(th)
            .then_translate(center)
    }

    /// An affine transform representing translation.
    #[inline(always)]
    pub const fn translate(p: (f64, f64)) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, p.0, p.1])
    }

    /// An affine transformation representing a skew.
    ///
    /// The `skew_x` and `skew_y` parameters represent skew factors for the
    /// horizontal and vertical directions, respectively.
    #[inline(always)]
    pub const fn skew(skew_x: f64, skew_y: f64) -> Affine {
        Affine([1.0, skew_y, skew_x, 1.0, 0.0, 0.0])
    }

    /// An affine transform that represents reflection about a line through `point`
    /// in the given `direction`.
    #[inline]
    #[must_use]
    pub fn reflect(point: (f64, f64), direction: (f64, f64)) -> Self {
        let hypot = (direction.0 * direction.0 + direction.1 * direction.1).sqrt();
        let nx = direction.1 / hypot;
        let ny = -direction.0 / hypot;

        let x2 = nx * nx;
        let xy = nx * ny;
        let y2 = ny * ny;
        let aff = Affine::new([
            1. - 2. * x2,
            -2. * xy,
            -2. * xy,
            1. - 2. * y2,
            point.0,
            point.1,
        ]);
        aff.pre_translate((-point.0, -point.1))
    }

    /// A [rotation] by `th` followed by `self`.
    ///
    /// Equivalent to `self * Affine::rotate(th)`
    ///
    /// [rotation]: Affine::rotate
    #[inline]
    #[must_use]
    pub fn pre_rotate(self, th: f64) -> Self {
        self * Affine::rotate(th)
    }

    /// A [rotation] by `th` about `center` followed by `self`.
    ///
    /// Equivalent to `Affine::rotate_about(th, center) * self`
    ///
    /// [rotation]: Affine::rotate_about
    #[inline]
    #[must_use]
    pub fn pre_rotate_about(self, th: f64, center: (f64, f64)) -> Self {
        Affine::rotate_about(th, center) * self
    }

    /// A [scale] by `scale` followed by `self`.
    ///
    /// Equivalent to `self * Affine::scale(scale)`
    ///
    /// [scale]: Affine::scale
    #[inline]
    #[must_use]
    pub fn pre_scale(self, scale: f64) -> Self {
        self * Affine::scale(scale)
    }

    /// A [scale] by `(scale_x, scale_y)` followed by `self`.
    ///
    /// Equivalent to `self * Affine::scale_non_uniform(scale_x, scale_y)`
    ///
    /// [scale]: Affine::scale_non_uniform
    #[inline]
    #[must_use]
    pub fn pre_scale_non_uniform(self, scale_x: f64, scale_y: f64) -> Self {
        self * Affine::scale_non_uniform(scale_x, scale_y)
    }

    /// A [translation] of `trans` followed by `self`.
    ///
    /// Equivalent to `self * Affine::translate(trans)`
    ///
    /// [translation]: Affine::translate
    #[inline]
    #[must_use]
    pub fn pre_translate(self, trans: (f64, f64)) -> Self {
        self * Affine::translate(trans)
    }

    /// `self` followed by a [rotation] of `th`.
    ///
    /// Equivalent to `Affine::rotate(th) * self`
    ///
    /// [rotation]: Affine::rotate
    #[inline]
    #[must_use]
    pub fn then_rotate(self, th: f64) -> Self {
        Affine::rotate(th) * self
    }

    /// `self` followed by a [rotation] of `th` about `center`.
    ///
    /// Equivalent to `Affine::rotate_about(th, center) * self`
    ///
    /// [rotation]: Affine::rotate_about
    #[inline]
    #[must_use]
    pub fn then_rotate_about(self, th: f64, center: (f64, f64)) -> Self {
        Affine::rotate_about(th, center) * self
    }

    /// `self` followed by a [scale] of `scale`.
    ///
    /// Equivalent to `Affine::scale(scale) * self`
    ///
    /// [scale]: Affine::scale
    #[inline]
    #[must_use]
    pub fn then_scale(self, scale: f64) -> Self {
        Affine::scale(scale) * self
    }

    /// `self` followed by a [scale] of `(scale_x, scale_y)`.
    ///
    /// Equivalent to `Affine::scale_non_uniform(scale_x, scale_y) * self`
    ///
    /// [scale]: Affine::scale_non_uniform
    #[inline]
    #[must_use]
    pub fn then_scale_non_uniform(self, scale_x: f64, scale_y: f64) -> Self {
        Affine::scale_non_uniform(scale_x, scale_y) * self
    }

    /// `self` followed by a [scale] of `scale` about `center`.
    ///
    /// Equivalent to `Affine::scale_about(scale, center) * self`
    ///
    /// [scale]: Affine::scale_about
    #[inline]
    #[must_use]
    pub fn then_scale_about(self, scale: f64, center: (f64, f64)) -> Self {
        Affine::scale_about(scale, center) * self
    }

    /// `self` followed by a translation of `trans`.
    ///
    /// Equivalent to `Affine::translate(trans) * self`
    #[inline]
    #[must_use]
    pub const fn then_translate(mut self, trans: (f64, f64)) -> Self {
        self.0[4] += trans.0;
        self.0[5] += trans.1;
        self
    }

    /// Creates an affine transformation that takes the unit square to the given rectangle.
    ///
    /// Useful when you want to draw into the unit square but have your output fill any rectangle.
    pub const fn map_unit_square(rect: Rect) -> Affine {
        Affine([rect.width(), 0., 0., rect.height(), rect.x0, rect.y0])
    }

    /// Get the coefficients of the transform.
    #[inline(always)]
    pub const fn as_coeffs(self) -> [f64; 6] {
        self.0
    }

    /// Compute the determinant of this transform.
    pub const fn determinant(self) -> f64 {
        self.0[0] * self.0[3] - self.0[1] * self.0[2]
    }

    /// Compute the inverse transform.
    ///
    /// Produces NaN values when the determinant is zero.
    pub const fn inverse(self) -> Affine {
        let inv_det = self.determinant().recip();
        Affine([
            inv_det * self.0[3],
            -inv_det * self.0[1],
            -inv_det * self.0[2],
            inv_det * self.0[0],
            inv_det * (self.0[2] * self.0[5] - self.0[3] * self.0[4]),
            inv_det * (self.0[1] * self.0[4] - self.0[0] * self.0[5]),
        ])
    }

    /// Compute the bounding box of a transformed rectangle.
    ///
    /// Returns the minimal `Rect` that encloses the given `Rect` after affine transformation.
    /// If the transform is axis-aligned, then this bounding box is "tight", in other words the
    /// returned `Rect` is the transformed rectangle.
    ///
    /// The returned rectangle always has non-negative width and height.
    pub fn transform_rect_bbox(self, rect: Rect) -> Rect {
        let p00 = self.transform_point(rect.x0, rect.y0);
        let p01 = self.transform_point(rect.x0, rect.y1);
        let p10 = self.transform_point(rect.x1, rect.y0);
        let p11 = self.transform_point(rect.x1, rect.y1);

        let min_x = p00.0.min(p01.0).min(p10.0).min(p11.0);
        let min_y = p00.1.min(p01.1).min(p10.1).min(p11.1);
        let max_x = p00.0.max(p01.0).max(p10.0).max(p11.0);
        let max_y = p00.1.max(p01.1).max(p10.1).max(p11.1);

        Rect::new(min_x, min_y, max_x, max_y)
    }

    /// Transform a point by this affine map.
    #[inline]
    pub fn transform_point(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.0[0] * x + self.0[2] * y + self.0[4],
            self.0[1] * x + self.0[3] * y + self.0[5],
        )
    }

    /// Is this map [finite]?
    ///
    /// [finite]: f64::is_finite
    #[inline]
    pub const fn is_finite(&self) -> bool {
        self.0[0].is_finite()
            && self.0[1].is_finite()
            && self.0[2].is_finite()
            && self.0[3].is_finite()
            && self.0[4].is_finite()
            && self.0[5].is_finite()
    }

    /// Is this map [NaN]?
    ///
    /// [NaN]: f64::is_nan
    #[inline]
    pub const fn is_nan(&self) -> bool {
        self.0[0].is_nan()
            || self.0[1].is_nan()
            || self.0[2].is_nan()
            || self.0[3].is_nan()
            || self.0[4].is_nan()
            || self.0[5].is_nan()
    }

    /// Returns the translation part of this affine map.
    #[inline(always)]
    pub const fn translation(self) -> (f64, f64) {
        (self.0[4], self.0[5])
    }

    /// Replaces the translation portion of this affine map.
    ///
    /// The translation can be seen as being applied after the linear part of the map.
    #[must_use]
    #[inline(always)]
    pub const fn with_translation(mut self, trans: (f64, f64)) -> Affine {
        self.0[4] = trans.0;
        self.0[5] = trans.1;
        self
    }
}

impl Default for Affine {
    #[inline(always)]
    fn default() -> Affine {
        Affine::IDENTITY
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

impl MulAssign for Affine {
    #[inline]
    fn mul_assign(&mut self, other: Affine) {
        *self = self.mul(other);
    }
}

impl Mul<Affine> for f64 {
    type Output = Affine;

    #[inline]
    fn mul(self, other: Affine) -> Affine {
        Affine([
            self * other.0[0],
            self * other.0[1],
            self * other.0[2],
            self * other.0[3],
            self * other.0[4],
            self * other.0[5],
        ])
    }
}
