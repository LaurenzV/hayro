//! A decoder for CCITT fax-encoded images.
//!
//! This crate implements the CCITT Group 3 and Group 4 fax compression algorithms
//! as defined in ITU-T Recommendations T.4 and T.6. These encodings are commonly
//! used for bi-level (black and white) images in PDF documents and fax transmissions.
//!
//! The main entry point is the [`decode`] function, which takes encoded data, a
//! [`DecoderContext`], and outputs the decoded pixels through a [`Decoder`] trait
//! that can be implemented according to your needs.
//!
//! The crate is `no_std` compatible but requires an allocator to be available.
//!
//! # Safety
//! Unsafe code is forbidden via a crate-level attribute.
//!
//! # License
//! Licensed under either of
//!
//! - Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
//! - MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
//!
//! at your option.
//!
//! [`decode`]: crate::decode
//! [`Decoder`]: crate::Decoder

#![no_std]
#![forbid(unsafe_code)]
#![forbid(missing_docs)]

extern crate alloc;

use crate::bit_reader::BitReader;

mod bit_reader;
mod codes;
mod context;
mod error;
mod group3;
mod group4;
mod tables;

pub use context::DecoderContext;
pub use error::{DecodeError, Result};

/// The encoding mode for CCITT fax decoding.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodingMode {
    /// Group 4 (MMR).
    Group4,
    /// Group 3 1D (MH).
    Group3_1D,
    /// Group 3 2D (MR).
    Group3_2D,
}

/// Settings to apply during decoding.
#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct DecodeSettings {
    /// How many columns the image has (i.e. its width).
    pub columns: u32,
    /// How many rows the image has (i.e. its height).
    ///
    /// In case `end_of_block` has been set to true, decoding will run until
    /// the given number of rows have been decoded, or the `end_of_block` marker
    /// has been encountered, whichever occurs first.
    ///
    /// If set to 0, decoding runs until the end-of-block marker or the end of the data.
    pub rows: u32,
    /// Whether the stream _MAY_ contain an end-of-block marker
    /// (It doesn't have to. In that case this is set to `true` but there are
    /// no end-of-block markers, hayro-ccitt will still use the value of `rows`
    /// to determine when to stop decoding).
    ///
    /// Defaults to `true`.
    pub end_of_block: bool,
    /// Whether the stream contains end-of-line markers.
    ///
    /// Defaults to `false`.
    pub end_of_line: bool,
    /// Whether the data in the stream for each row is aligned to the byte
    /// boundary.
    ///
    /// Defaults to `false`.
    pub rows_are_byte_aligned: bool,
    /// The encoding mode used by the image.
    ///
    /// Defaults to [`EncodingMode::Group3_1D`].
    pub encoding: EncodingMode,
    /// Whether black and white should be inverted.
    ///
    /// Defaults to `false`.
    pub invert_black: bool,
    /// Whether to reject data that violates the specification, instead of
    /// decoding it as well as possible.
    ///
    /// Defaults to `false`.
    pub strict: bool,
}

impl DecodeSettings {
    /// Creates new settings for an image with the given number of columns and
    /// rows, using the default values for all other settings.
    pub fn new(columns: u32, rows: u32) -> Self {
        Self {
            columns,
            rows,
            end_of_block: true,
            end_of_line: false,
            rows_are_byte_aligned: false,
            encoding: EncodingMode::Group3_1D,
            invert_black: false,
            strict: false,
        }
    }
}

/// A decoder for CCITT images.
pub trait Decoder {
    /// Push a run of pixels of the same color.
    fn push_pixels(&mut self, white: bool, count: u32);
    /// Called when a row has been completed.
    fn next_line(&mut self);
}

/// Decode the given image using the provided decoder context and decoder.
///
/// If decoding was successful, the number of bytes that have been read in total
/// is returned.
///
/// If an error is returned, it means that the file is somehow malformed.
/// However, even if that's the case, it is possible that a number
/// of rows were decoded successfully and written into the decoder, so those
/// can still be used, but the image might be truncated.
pub fn decode(data: &[u8], decoder: &mut impl Decoder, ctx: &mut DecoderContext) -> Result<usize> {
    ctx.reset();
    let mut reader = BitReader::new(data);

    if ctx.settings.columns == 0 {
        return Ok(0);
    }

    match ctx.settings.encoding {
        EncodingMode::Group4 => group4::decode(ctx, &mut reader, decoder)?,
        EncodingMode::Group3_1D => group3::decode_1d(ctx, &mut reader, decoder)?,
        EncodingMode::Group3_2D => group3::decode_2d(ctx, &mut reader, decoder)?,
    }

    reader.align();
    Ok(reader.byte_pos())
}
