//! A decoder for CCITT fax-encoded images.
//!
//! This crate implements the CCITT Group 3 and Group 4 fax compression algorithms
//! as defined in ITU-T Recommendations T.4 and T.6. These encodings are commonly
//! used for bi-level (black and white) images in PDF documents and fax transmissions.
//!
//! The main entry point is the [`decode`] function, which takes encoded data and
//! [`DecodeSettings`], and outputs each decoded row as a list of color changes
//! through a [`Decoder`] trait that can be implemented according to your needs.
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
//! [`DecodeSettings`]: crate::DecodeSettings

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![forbid(missing_docs)]

extern crate alloc;

use crate::bit_reader::BitReader;
use crate::context::DecoderContext;
use crate::sync::Arc;

mod bit_reader;
mod cancel;
mod codes;
mod context;
mod error;
mod group3;
mod group4;
mod sync;
mod tables;

pub use cancel::CancelCheck;
pub use error::{DecodeError, Result};

/// Decode the given image using the provided settings and decoder.
///
/// If decoding was successful, the number of bytes that have been read in total
/// is returned.
///
/// If an error is returned, it means that the file is somehow malformed.
/// However, even if that's the case, it is possible that a number
/// of rows were decoded successfully and written into the decoder, so those
/// can still be used, but the image might be truncated.
pub fn decode(data: &[u8], decoder: &mut impl Decoder, settings: &DecodeSettings) -> Result<usize> {
    if settings.columns == 0 {
        return Ok(0);
    }

    let mut reader = BitReader::new(data);
    let mut ctx = DecoderContext::new(settings);

    match settings.encoding {
        EncodingMode::Group4 => group4::decode(&mut ctx, &mut reader, decoder)?,
        EncodingMode::Group3_1D => group3::decode_1d(&mut ctx, &mut reader, decoder)?,
        EncodingMode::Group3_2D => group3::decode_2d(&mut ctx, &mut reader, decoder)?,
    }

    reader.align();
    Ok(reader.byte_pos())
}

/// A sink for the rows of a CCITT image.
pub trait Decoder {
    /// Push a decoded row.
    ///
    /// `changes` contains the positions at which the color changes, in strictly
    /// ascending order and each smaller than the number of columns. Each row
    /// starts white, so the pixels from `changes[0]` up to `changes[1]` are
    /// black, those from `changes[2]` up to `changes[3]` again, and so on. If
    /// the number of changes is odd, the row ends black.
    fn push_row(&mut self, changes: &[u32]);
}

/// Settings to apply during decoding.
#[derive(Clone)]
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
    pub end_of_block: bool,
    /// Whether the stream contains end-of-line markers.
    pub end_of_line: bool,
    /// Whether the data in the stream for each row is aligned to the byte
    /// boundary.
    pub rows_are_byte_aligned: bool,
    /// The encoding mode used by the image.
    pub encoding: EncodingMode,
    /// Whether to reject data that violates the specification, instead of
    /// decoding it as well as possible.
    pub strict: bool,
    /// A check for cancelling decoding.
    pub cancel: Option<Arc<dyn CancelCheck>>,
}

impl core::fmt::Debug for DecodeSettings {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecodeSettings")
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .field("end_of_block", &self.end_of_block)
            .field("end_of_line", &self.end_of_line)
            .field("rows_are_byte_aligned", &self.rows_are_byte_aligned)
            .field("encoding", &self.encoding)
            .field("strict", &self.strict)
            .finish_non_exhaustive()
    }
}

/// The encoding mode for CCITT fax decoding.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EncodingMode {
    /// Group 4 (MMR).
    Group4,
    /// Group 3 1D (MH).
    Group3_1D,
    /// Group 3 2D (MR).
    Group3_2D,
}
