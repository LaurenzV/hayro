// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use bytemuck::{Pod, Zeroable};
use hayro_interpret::color::AlphaColor;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use std::io::Cursor;
use std::vec;
use std::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
#[repr(C)]
pub(crate) struct PremulRgba8 {
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
    #[must_use]
    pub(crate) const fn from_u8_array([r, g, b, a]: [u8; 4]) -> Self {
        Self { r, g, b, a }
    }

    #[must_use]
    pub(crate) const fn from_u32(packed_bytes: u32) -> Self {
        Self::from_u8_array(u32::to_ne_bytes(packed_bytes))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
#[repr(C)]
pub(crate) struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
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
    pub(crate) fn new(width: u16, height: u16) -> Self {
        let buf = vec![PremulRgba8::from_u32(0); width as usize * height as usize];
        Self { width, height, buf }
    }

    /// Return the width of the pixmap.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Return the height of the pixmap.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Returns a reference to the underlying data as premultiplied RGBA8.
    ///
    /// The pixels are in row-major order.
    pub(crate) fn data(&self) -> &[PremulRgba8] {
        &self.buf
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

    /// Consume the pixmap, returning the data as the underlying [`Vec`] of premultiplied RGBA8.
    ///
    /// The pixels are in row-major order.
    pub fn take_u8(self) -> Vec<u8> {
        bytemuck::cast_vec(self.buf)
    }

    /// Encode the pixmap into a PNG file.
    pub fn take_png(self) -> Vec<u8> {
        let mut png_data = Vec::new();
        let cursor = Cursor::new(&mut png_data);
        let encoder = PngEncoder::new(cursor);
        encoder
            .write_image(
                self.data_as_u8_slice(),
                self.width() as u32,
                self.height() as u32,
                ExtendedColorType::Rgba8,
            )
            .expect("Failed to encode image");

        png_data
    }
}
