// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Types for paints.

use crate::encode::Buffer;
use hayro_interpret::color::AlphaColor;
use hayro_interpret::pattern::ShadingPattern;
use kurbo::Affine;
use std::sync::Arc;

/// A paint that needs to be resolved via its index.
// In the future, we might add additional flags, that's why we have
// this thin wrapper around u32, so we can change the underlying
// representation without breaking the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedPaint(u32);

impl IndexedPaint {
    /// Create a new indexed paint from an index.
    pub fn new(index: usize) -> Self {
        Self(u32::try_from(index).expect("exceeded the maximum number of paints"))
    }

    /// Return the index of the paint.
    pub fn index(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

/// A paint that is used internally by a rendering frontend to store how a wide tile command
/// should be painted. There are only two types of paint:
///
/// 1) Simple solid colors, which are stored in premultiplied representation so that
///    each wide tile doesn't have to recompute it.
/// 2) Indexed paints, which can represent any arbitrary, more complex paint that is
///    determined by the frontend. The intended way of using this is to store a vector
///    of paints and store its index inside `IndexedPaint`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Paint {
    /// A premultiplied RGBA8 color.
    Solid(PremulColor),
    /// A paint that needs to be resolved via an index.
    Indexed(IndexedPaint),
}

impl From<AlphaColor> for Paint {
    fn from(value: AlphaColor) -> Self {
        Self::Solid(PremulColor::from_alpha_color(value))
    }
}

/// An image.
#[derive(Debug, Clone)]
pub(crate) struct Image {
    pub buffer: Arc<Buffer<4>>,
    pub interpolate: bool,
    pub is_stencil: bool,
    pub is_pattern: bool,
    pub transform: Affine,
}

/// A premultiplied color.
#[derive(Debug, Clone, PartialEq, Copy)]
pub(crate) struct PremulColor(pub [f32; 4]);

impl PremulColor {
    /// Create a new premultiplied color.
    pub fn from_alpha_color(color: AlphaColor) -> Self {
        Self(color.premultiplied())
    }

    /// Return whether the color is opaque (i.e. doesn't have transparency).
    pub fn is_opaque(&self) -> bool {
        self.0[3] == 1.0
    }
}

/// A kind of paint that can be used for filling and stroking shapes.
#[derive(Debug, Clone)]
pub(crate) enum PaintType {
    /// A solid color.
    Solid(AlphaColor),
    /// An image.
    Image(Image),
    /// A shading pattern.
    ShadingPattern(ShadingPattern),
}

impl From<AlphaColor> for PaintType {
    fn from(value: AlphaColor) -> Self {
        Self::Solid(value)
    }
}

impl From<ShadingPattern> for PaintType {
    fn from(value: ShadingPattern) -> Self {
        Self::ShadingPattern(value)
    }
}

impl From<Image> for PaintType {
    fn from(value: Image) -> Self {
        Self::Image(value)
    }
}
