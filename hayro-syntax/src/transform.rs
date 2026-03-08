// Copyright 2018 the Kurbo Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Affine transforms.

use core::ops::{Mul, MulAssign};

use crate::{Point, Rect, Vec2};

#[cfg(not(feature = "std"))]
use crate::common::FloatFuncs;

/// A 2D affine transform.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// with different scale values for x and y
    #[inline(always)]
    pub const fn scale_non_uniform(s_x: f64, s_y: f64) -> Affine {
        Affine([s_x, 0.0, 0.0, s_y, 0.0, 0.0])
    }

    /// An affine transform representing a scale of `scale` about `center`.
    ///
    /// Useful for a view transform that zooms at a specific point,
    /// while keeping that point fixed in the result space.
    ///
    /// See [`Affine::scale()`] for more info.
    #[inline]
    pub fn scale_about(s: f64, center: impl Into<Point>) -> Affine {
        let center = center.into().to_vec2();
        Self::translate(-center)
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
    ///
    /// See [`Affine::rotate()`] for more info.
    #[inline]
    pub fn rotate_about(th: f64, center: impl Into<Point>) -> Affine {
        let center = center.into().to_vec2();
        Self::translate(-center)
            .then_rotate(th)
            .then_translate(center)
    }

    /// An affine transform representing translation.
    #[inline(always)]
    pub fn translate<V: Into<Vec2>>(p: V) -> Affine {
        let p = p.into();
        Affine([1.0, 0.0, 0.0, 1.0, p.x, p.y])
    }

    /// An affine transformation representing a skew.
    ///
    /// The `skew_x` and `skew_y` parameters represent skew factors for the
    /// horizontal and vertical directions, respectively.
    ///
    /// This is commonly used to generate a faux oblique transform for
    /// font rendering. In this case, you can slant the glyph 20 degrees
    /// clockwise in the horizontal direction (assuming a Y-up coordinate
    /// system):
    ///
    /// ```
    /// let oblique_transform = kurbo::Affine::skew(20f64.to_radians().tan(), 0.0);
    /// ```
    #[inline(always)]
    pub const fn skew(skew_x: f64, skew_y: f64) -> Affine {
        Affine([1.0, skew_y, skew_x, 1.0, 0.0, 0.0])
    }

    /// Create an affine transform that represents reflection about the line `point + direction * t, t in (-infty, infty)`
    ///
    /// # Examples
    ///
    /// ```
    /// # use kurbo::{Point, Vec2, Affine};
    /// # fn assert_near(p0: Point, p1: Point) {
    /// #     assert!((p1 - p0).hypot() < 1e-9, "{p0:?} != {p1:?}");
    /// # }
    /// let point = Point::new(1., 0.);
    /// let vec = Vec2::new(1., 1.);
    /// let map = Affine::reflect(point, vec);
    /// assert_near(map * Point::new(1., 0.), Point::new(1., 0.));
    /// assert_near(map * Point::new(2., 1.), Point::new(2., 1.));
    /// assert_near(map * Point::new(2., 2.), Point::new(3., 1.));
    /// ```
    #[inline]
    #[must_use]
    pub fn reflect(point: impl Into<Point>, direction: impl Into<Vec2>) -> Self {
        let point = point.into();
        let direction = direction.into();

        let n = Vec2 {
            x: direction.y,
            y: -direction.x,
        }
            .normalize();

        // Compute Householder reflection matrix
        let x2 = n.x * n.x;
        let xy = n.x * n.y;
        let y2 = n.y * n.y;
        // Here we also add in the post translation, because it doesn't require any further calc.
        let aff = Affine::new([
            1. - 2. * x2,
            -2. * xy,
            -2. * xy,
            1. - 2. * y2,
            point.x,
            point.y,
        ]);
        aff.pre_translate(-point.to_vec2())
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
    /// Equivalent to `self * Affine::rotate_about(th, center)`
    ///
    /// [rotation]: Affine::rotate_about
    #[inline]
    #[must_use]
    pub fn pre_rotate_about(self, th: f64, center: impl Into<Point>) -> Self {
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
    pub fn pre_translate(self, trans: Vec2) -> Self {
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
    pub fn then_rotate_about(self, th: f64, center: impl Into<Point>) -> Self {
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
    /// Equivalent to `Affine::scale_about(scale) * self`
    ///
    /// [scale]: Affine::scale_about
    #[inline]
    #[must_use]
    pub fn then_scale_about(self, scale: f64, center: impl Into<Point>) -> Self {
        Affine::scale_about(scale, center) * self
    }

    /// `self` followed by a translation of `trans`.
    ///
    /// Equivalent to `Affine::translate(trans) * self`
    ///
    /// [translation]: Affine::translate
    #[inline]
    #[must_use]
    pub const fn then_translate(mut self, trans: Vec2) -> Self {
        self.0[4] += trans.x;
        self.0[5] += trans.y;
        self
    }

    /// Creates an affine transformation that takes the unit square to the given rectangle.
    ///
    /// Useful when you want to draw into the unit square but have your output fill any rectangle.
    /// In this case push the `Affine` onto the transform stack.
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
        let p00 = self * Point::new(rect.x0, rect.y0);
        let p01 = self * Point::new(rect.x0, rect.y1);
        let p10 = self * Point::new(rect.x1, rect.y0);
        let p11 = self * Point::new(rect.x1, rect.y1);
        Rect::from_points(p00, p01).union(Rect::from_points(p10, p11))
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

    /// Compute the singular value decomposition of the linear transformation (ignoring the
    /// translation).
    ///
    /// All non-degenerate linear transformations can be represented as
    ///
    ///  1. a rotation about the origin.
    ///  2. a scaling along the x and y axes
    ///  3. another rotation about the origin
    ///
    /// composed together. Decomposing a 2x2 matrix in this way is called a "singular value
    /// decomposition" and is written `U Σ V^T`, where U and V^T are orthogonal (rotations) and Σ
    /// is a diagonal matrix (a scaling).
    ///
    /// Since currently this function is used to calculate ellipse radii and rotation from an
    /// affine map on the unit circle, we don't calculate V^T, since a rotation of the unit (or
    /// any) circle about its center always results in the same circle. This is the reason that an
    /// ellipse mapped using an affine map is always an ellipse.
    ///
    /// Will return NaNs if the matrix (or equivalently the linear map) is non-finite.
    ///
    /// The first part of the returned tuple is the scaling, the second part is the angle of
    /// rotation (in radians). The scaling along the x-axis is guaranteed to be greater than or
    /// equal to the scaling along the y-axis.
    //
    // Note: though this does quite some computation, we are often interested only in specific
    // components of the result. Hence this is marked `#[inline(always)]`, to give the compiler a
    // good chance at eliminating dead code.
    #[inline(always)]
    pub(crate) fn svd(self) -> (Vec2, f64) {
        let [a, b, c, d, _, _] = self.0;
        let a2 = a * a;
        let b2 = b * b;
        let c2 = c * c;
        let d2 = d * d;
        let ab = a * b;
        let cd = c * d;
        let angle = 0.5 * (2.0 * (ab + cd)).atan2(a2 - b2 + c2 - d2);

        // Given matrix A = [ a c ]
        //                  [ b d ]
        //
        // The two singular values σ1, σ2 of A are the square roots of the two eigen values λ1, λ2
        // of M = A^T A. The common formula for 2x2 eigenvalues requires evaluating a square root,
        // but we'd like to compute the singular values of the matrix without nested square roots.
        //
        // M = A^T A = [ aa+cc   ab+cd ]
        //             [ ab+cd   bb+dd ]
        //
        // We have
        // λ = 1/2 (tr(M) ± sqrt(tr(M)^2 - 4 det(M))).
        //
        // Note det(M) = det(A^T A) = det(A)^2.
        // => 2λ = tr(M) ± sqrt(tr(M)^2 - 4 det(A)^2)
        // => 2λ = tr(M) ± sqrt[(a^2+b^2+c^2+d^2)^2 - 4 (ad-bc)^2]
        // By factorizing the inner term,
        // => 2λ = tr(M) ± sqrt[((a+d)^2 + (b-c)^2) ((a-d)^2 + (b+c)^2)]
        // => 2λ = tr(M) ± sqrt[(a+d)^2 + (b-c)^2] sqrt[(a-d)^2 + (b+c)^2]
        //
        // Define S1 = sqrt[(a+d)^2 + (b-c)^2]
        //        S2 = sqrt[(a-d)^2 + (b+c)^2].
        //
        // => 2λ = tr(M) ± S1 S2
        // => 2λ = 1/2 (S1^2 + S2^2) ± S1 S2
        // => λ = 1/4 (S1^2 + S2^2 ± 2 S1 S2)
        // => λ = 1/4 (S1 ± S2)^2
        //
        // Note we're interested in
        // σ = sqrt(λ).
        //
        // => σ1 = 1/2 (S1 + S2)
        // and similarly σ2 = 1/2 |S1 - S2|
        let s1 = ((a + d).powi(2) + (b - c).powi(2)).sqrt();
        let s2 = ((a - d).powi(2) + (b + c).powi(2)).sqrt();
        (
            Vec2 {
                x: 0.5 * (s1 + s2),
                y: 0.5 * (s1 - s2).abs(),
            },
            angle,
        )
    }

    /// Returns the translation part of this affine map (`(self.0[4], self.0[5])`).
    #[inline(always)]
    pub const fn translation(self) -> Vec2 {
        Vec2 {
            x: self.0[4],
            y: self.0[5],
        }
    }

    /// Replaces the translation portion of this affine map
    ///
    /// The translation can be seen as being applied after the linear part of the map.
    #[must_use]
    #[inline(always)]
    pub const fn with_translation(mut self, trans: Vec2) -> Affine {
        self.0[4] = trans.x;
        self.0[5] = trans.y;
        self
    }
}

impl Default for Affine {
    #[inline(always)]
    fn default() -> Affine {
        Affine::IDENTITY
    }
}

impl Mul<Point> for Affine {
    type Output = Point;

    #[inline]
    fn mul(self, other: Point) -> Point {
        Point::new(
            self.0[0] * other.x + self.0[2] * other.y + self.0[4],
            self.0[1] * other.x + self.0[3] * other.y + self.0[5],
        )
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
