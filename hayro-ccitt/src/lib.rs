//! A decoder for CCITT fax-encoded images.
//!
//! This crate implements the CCITT Group 3 and Group 4 fax compression algorithms
//! as defined in ITU-T Recommendations T.4 and T.6. These encodings are commonly
//! used for bi-level (black and white) images in PDF documents and fax transmissions.
//!
//! The main entry point is the [`decode`] function, which takes encoded data, a
//! [`DecoderContext`], and outputs each decoded row as a list of color changes
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

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![forbid(missing_docs)]

extern crate alloc;

use crate::bit_reader::BitReader;
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
mod util;

pub use cancel::CancelCheck;
pub use context::DecoderContext;
pub use error::{DecodeError, Result};

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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[derive(Default)]
    struct RowCollector {
        rows: Vec<Vec<u32>>,
    }

    impl Decoder for RowCollector {
        fn push_row(&mut self, changes: &[u32]) {
            self.rows.push(changes.to_vec());
        }
    }

    #[test]
    fn lenient_disables_byte_align_on_nonzero_padding() {
        // Row 0: V(0) = `1` (all 4 pixels white).
        // Row 1 (immediately packed without zero-padding): V(-2) = `000010`, V(0) = `1`
        // (2 white pixels, 2 black pixels -> changes = [2]).
        // Combined bits: `1_000010_1` = 0x85.
        let data = [0x85];

        let mut lenient = RowCollector::default();
        let mut lenient_ctx = DecoderContext::new(DecodeSettings {
            columns: 4,
            rows: 2,
            end_of_block: false,
            end_of_line: false,
            rows_are_byte_aligned: true,
            encoding: EncodingMode::Group4,
            strict: false,
            cancel: None,
        });
        assert_eq!(decode(&data, &mut lenient, &mut lenient_ctx), Ok(1));
        assert_eq!(lenient.rows, vec![vec![], vec![2]]);

        let mut strict = RowCollector::default();
        let mut strict_ctx = DecoderContext::new(DecodeSettings {
            strict: true,
            ..lenient_ctx.settings.clone()
        });
        assert_eq!(decode(&data, &mut strict, &mut strict_ctx), Ok(1));
        assert_eq!(strict.rows, vec![vec![]]);
    }

    #[test]
    fn lenient_group4_flushes_white_padded_partial_row_on_error() {
        // Row 0: V(0) = `1` (4 white pixels).
        // Row 1: V(-3) = `0000010` (1 white pixel), V(-1) = `010` (2 black pixels),
        // followed by invalid mode `0000000...` before the 4th pixel is decoded.
        let data = [0x82, 0x40, 0x00];

        let mut lenient = RowCollector::default();
        let mut lenient_ctx = DecoderContext::new(DecodeSettings {
            columns: 4,
            rows: 2,
            end_of_block: false,
            end_of_line: false,
            rows_are_byte_aligned: false,
            encoding: EncodingMode::Group4,
            strict: false,
            cancel: None,
        });
        assert_eq!(
            decode(&data, &mut lenient, &mut lenient_ctx),
            Err(DecodeError::InvalidCode)
        );
        assert_eq!(lenient.rows, vec![vec![], vec![1, 3]]);

        let mut strict = RowCollector::default();
        let mut strict_ctx = DecoderContext::new(DecodeSettings {
            strict: true,
            ..lenient_ctx.settings.clone()
        });
        assert_eq!(
            decode(&data, &mut strict, &mut strict_ctx),
            Err(DecodeError::InvalidCode)
        );
        assert_eq!(strict.rows, vec![vec![]]);
    }
}
