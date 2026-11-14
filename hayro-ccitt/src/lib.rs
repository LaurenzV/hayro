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

mod bit_reader;
mod cancel;
mod codes;
mod context;
mod error;
mod group3;
mod group4;
mod sync;
mod tables;

pub use cancel::{CancelCheck, CancelHandle};
pub use error::{DecodeError, Result};

/// Decode `data` into `decoder`, returning the number of bytes read.
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
    /// Push a row as the ascending positions where its color changes, starting from white.
    fn push_row(&mut self, changes: &[u32]);
}

/// Settings to apply during decoding.
#[derive(Debug, Clone)]
pub struct DecodeSettings {
    /// The width of the image.
    pub columns: u32,
    /// The height of the image, or 0 to decode until an end-of-block marker or the end of data.
    pub rows: u32,
    /// Whether the stream may contain an end-of-block marker.
    pub end_of_block: bool,
    /// Whether the stream contains end-of-line markers.
    pub end_of_line: bool,
    /// Whether each row starts at a byte boundary.
    pub rows_are_byte_aligned: bool,
    /// The encoding mode used by the image.
    pub encoding: EncodingMode,
    /// Whether to reject malformed data instead of decoding it leniently.
    pub strict: bool,
    /// A check for cancelling decoding.
    pub cancel: Option<CancelHandle>,
}

/// A CCITT encoding mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EncodingMode {
    /// Group 4 (MMR).
    Group4,
    /// Group 3 1D (MH).
    Group3_1D,
    /// Group 3 2D (MR).
    Group3_2D,
}
