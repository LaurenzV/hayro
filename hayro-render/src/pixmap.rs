// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A simple pixmap type.

use bytemuck::{Pod, Zeroable};
use hayro_interpret::color::AlphaColor;
use image::{ImageBuffer, Rgba};
use std::vec;
use std::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[repr(C)]
pub struct PremulRgba8 {
    /// Red component.
    pub r: u8,
    /// Green component.
    pub g: u8,
    /// Blue component.
    pub b: u8,
    /// Alpha component.
    pub a: u8,
}

impl PremulRgba8 {
    /// Returns the color as a `[u8; 4]`.
    ///
    /// The color values will be in the order `[r, g, b, a]`.
    #[must_use]
    pub const fn to_u8_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Convert the `[u8; 4]` byte array into a `PremulRgba8` color.
    ///
    /// The color values must be given in the order `[r, g, b, a]`.
    #[must_use]
    pub const fn from_u8_array([r, g, b, a]: [u8; 4]) -> Self {
        Self { r, g, b, a }
    }

    /// Returns the color as a little-endian packed value, with `r` the least significant byte and
    /// `a` the most significant.
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        u32::from_ne_bytes(self.to_u8_array())
    }

    /// Interpret the little-endian packed value as a color, with `r` the least significant byte
    /// and `a` the most significant.
    #[must_use]
    pub const fn from_u32(packed_bytes: u32) -> Self {
        Self::from_u8_array(u32::to_ne_bytes(packed_bytes))
    }
}

/// A packed representation of sRGB colors.
///
/// Encoding sRGB with 8 bits per component is extremely common, as
/// it is efficient and convenient, even if limited in accuracy and
/// gamut.
///
/// This is not meant to be a general purpose color type and is
/// intended for use with [`AlphaColor::to_rgba8`] and [`OpaqueColor::to_rgba8`].
///
/// For a pre-multiplied packed representation, see [`PremulRgba8`].
///
/// [`AlphaColor::to_rgba8`]: crate::AlphaColor::to_rgba8
/// [`OpaqueColor::to_rgba8`]: crate::OpaqueColor::to_rgba8
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[repr(C)]
pub struct Rgba8 {
    /// Red component.
    pub r: u8,
    /// Green component.
    pub g: u8,
    /// Blue component.
    pub b: u8,
    /// Alpha component.
    ///
    /// Alpha is interpreted as separated alpha.
    pub a: u8,
}

impl Rgba8 {
    /// Returns the color as a `[u8; 4]`.
    ///
    /// The color values will be in the order `[r, g, b, a]`.
    #[must_use]
    pub const fn to_u8_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Convert the `[u8; 4]` byte array into an `Rgba8` color.
    ///
    /// The color values must be given in the order `[r, g, b, a]`.
    #[must_use]
    pub const fn from_u8_array([r, g, b, a]: [u8; 4]) -> Self {
        Self { r, g, b, a }
    }

    /// Returns the color as a little-endian packed value, with `r` the least significant byte and
    /// `a` the most significant.
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        u32::from_ne_bytes(self.to_u8_array())
    }

    /// Interpret the little-endian packed value as a color, with `r` the least significant byte
    /// and `a` the most significant.
    #[must_use]
    pub const fn from_u32(packed_bytes: u32) -> Self {
        Self::from_u8_array(u32::to_ne_bytes(packed_bytes))
    }
}

impl From<Rgba8> for AlphaColor {
    fn from(value: Rgba8) -> Self {
        Self::from_rgba8(value.r, value.g, value.b, value.a)
    }
}

/// A pixmap of premultiplied RGBA8 values backed by [`u8`][core::u8].
#[derive(Debug, Clone)]
pub struct Pixmap {
    /// Width of the pixmap in pixels.  
    width: u16,
    /// Height of the pixmap in pixels.
    height: u16,
    /// Buffer of the pixmap in RGBA8 format.
    buf: Vec<PremulRgba8>,
}

impl Pixmap {
    /// Create a new pixmap with the given width and height in pixels.
    pub fn new(width: u16, height: u16) -> Self {
        let buf = vec![PremulRgba8::from_u32(0); width as usize * height as usize];
        Self { width, height, buf }
    }

    /// Create a new pixmap with the given premultiplied RGBA8 data.
    ///
    /// The `data` vector must be of length `width * height` exactly.
    ///
    /// The pixels are in row-major order.
    ///
    /// # Panics
    ///
    /// Panics if the `data` vector is not of length `width * height`.
    pub fn from_parts(data: Vec<PremulRgba8>, width: u16, height: u16) -> Self {
        assert_eq!(
            data.len(),
            usize::from(width) * usize::from(height),
            "Expected `data` to have length of exactly `width * height`"
        );
        Self {
            width,
            height,
            buf: data,
        }
    }

    /// Resizes the pixmap container to the given width and height; this does not resize the
    /// contained image.
    ///
    /// If the pixmap buffer has to grow to fit the new size, those pixels are set to transparent
    /// black. If the pixmap buffer is larger than required, the buffer is truncated and its
    /// reserved capacity is unchanged.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.buf.resize(
            usize::from(width) * usize::from(height),
            PremulRgba8::from_u32(0),
        );
    }

    /// Shrink the capacity of the pixmap buffer to fit the pixmap's current size.
    pub fn shrink_to_fit(&mut self) {
        self.buf.shrink_to_fit();
    }

    /// The reserved capacity (in pixels) of this pixmap.
    ///
    /// When calling [`Pixmap::resize`] with a `width * height` smaller than this value, the pixmap
    /// does not need to reallocate.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Return the width of the pixmap.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Return the height of the pixmap.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Apply an alpha value to the whole pixmap.
    pub fn multiply_alpha(&mut self, alpha: u8) {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "cannot overflow in this case"
        )]
        let multiply = |component| ((u16::from(alpha) * u16::from(component)) / 255) as u8;

        for pixel in self.data_mut() {
            *pixel = PremulRgba8 {
                r: multiply(pixel.r),
                g: multiply(pixel.g),
                b: multiply(pixel.b),
                a: multiply(pixel.a),
            };
        }
    }

    /// Returns a reference to the underlying data as premultiplied RGBA8.
    ///
    /// The pixels are in row-major order.
    pub fn data(&self) -> &[PremulRgba8] {
        &self.buf
    }

    /// Returns a mutable reference to the underlying data as premultiplied RGBA8.
    ///
    /// The pixels are in row-major order.
    pub fn data_mut(&mut self) -> &mut [PremulRgba8] {
        &mut self.buf
    }

    /// Returns a reference to the underlying data as premultiplied RGBA8.
    ///
    /// The pixels are in row-major order. Each pixel consists of four bytes in the order
    /// `[r, g, b, a]`.
    pub fn data_as_u8_slice(&self) -> &[u8] {
        bytemuck::cast_slice(&self.buf)
    }

    /// Returns a mutable reference to the underlying data as premultiplied RGBA8.
    ///
    /// The pixels are in row-major order. Each pixel consists of four bytes in the order
    /// `[r, g, b, a]`.
    pub fn data_as_u8_slice_mut(&mut self) -> &mut [u8] {
        bytemuck::cast_slice_mut(&mut self.buf)
    }

    /// Sample a pixel from the pixmap.
    ///
    /// The pixel data is [premultiplied RGBA8][PremulRgba8].
    #[inline(always)]
    pub fn sample(&self, x: u16, y: u16) -> PremulRgba8 {
        let idx = self.width as usize * y as usize + x as usize;
        self.buf[idx]
    }

    /// Consume the pixmap, returning the data as the underlying [`Vec`] of premultiplied RGBA8.
    ///
    /// The pixels are in row-major order.
    pub fn take(self) -> Vec<PremulRgba8> {
        self.buf
    }

    /// Consume the pixmap, returning the data as (unpremultiplied) RGBA8.
    ///
    /// Not fast, but useful for saving to PNG etc.
    ///
    /// The pixels are in row-major order.
    pub fn take_unpremultiplied(self) -> Vec<Rgba8> {
        self.buf
            .into_iter()
            .map(|PremulRgba8 { r, g, b, a }| {
                let alpha = 255.0 / f32::from(a);
                if a != 0 {
                    #[expect(clippy::cast_possible_truncation, reason = "deliberate quantization")]
                    let unpremultiply = |component| (f32::from(component) * alpha + 0.5) as u8;
                    Rgba8 {
                        r: unpremultiply(r),
                        g: unpremultiply(g),
                        b: unpremultiply(b),
                        a,
                    }
                } else {
                    Rgba8 { r, g, b, a }
                }
            })
            .collect()
    }

    pub fn save_png(self, path: impl AsRef<std::path::Path>) {
        let width = self.width as u32;
        let height = self.height as u32;
        let data = self.take_unpremultiplied();
        let as_u8: &[u8] = bytemuck::cast_slice(&data);

        let image: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(width, height, as_u8).unwrap();
        image.save(path).unwrap()
    }
}
