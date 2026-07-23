//! Results from decoding filtered data streams.

mod ascii_85;
pub(crate) mod ascii_hex;
#[cfg(feature = "images")]
mod ccitt;
#[cfg(feature = "images")]
mod dct;
#[cfg(feature = "images")]
mod jbig2;
#[cfg(feature = "images")]
mod jpx;
mod lzw_flate;
mod png;
mod run_length;

use crate::object::Dict;
use crate::object::Name;
use crate::object::dict::keys::*;
use crate::object::stream::{DecodeFailure, FilterResult, ImageDecodeParams};
use alloc::sync::Arc;
use core::ops::Deref;
use enough::Stop;

/// Throttles cooperative-cancellation polling: checks the stop at most once per
/// `interval` bytes of produced output. `enough` leaves poll throttling to the
/// caller, and the byte-oriented decoders all share this pattern.
pub(crate) struct Pace {
    interval: usize,
    next: usize,
}

impl Pace {
    pub(crate) fn new(interval: usize) -> Self {
        Self {
            interval,
            next: interval,
        }
    }

    /// If at least `interval` bytes were produced since the last checkpoint,
    /// poll `stop`, propagating [`DecodeFailure::Stopped`] if it fires. Pass a
    /// `stop` already gated by `may_stop().then_some(..)` so an unstoppable
    /// decode skips the check entirely.
    pub(crate) fn poll(
        &mut self,
        produced: usize,
        stop: Option<&dyn Stop>,
    ) -> Result<(), DecodeFailure> {
        if produced >= self.next {
            self.next = produced.saturating_add(self.interval);
            stop.check()?;
        }
        Ok(())
    }
}

/// A data filter.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Filter {
    /// ASCII hexadecimal encoding.
    AsciiHexDecode,
    /// ASCII base-85 encoding.
    Ascii85Decode,
    /// Lempel-Ziv-Welch (LZW) compression.
    LzwDecode,
    /// DEFLATE compression (zlib/gzip).
    FlateDecode,
    /// Run-length encoding compression.
    RunLengthDecode,
    /// CCITT Group 3 or Group 4 fax compression.
    CcittFaxDecode,
    /// JBIG2 compression for bi-level images.
    Jbig2Decode,
    /// JPEG (DCT) compression.
    DctDecode,
    /// JPEG 2000 compression.
    JpxDecode,
    /// Encryption filter.
    Crypt,
}

impl Filter {
    fn debug_name(&self) -> &'static str {
        match self {
            Self::AsciiHexDecode => "ascii_hex",
            Self::Ascii85Decode => "ascii_85",
            Self::LzwDecode => "lzw",
            Self::FlateDecode => "flate",
            Self::RunLengthDecode => "run-length",
            Self::CcittFaxDecode => "ccit_fax",
            Self::Jbig2Decode => "jbig2",
            Self::DctDecode => "dct",
            Self::JpxDecode => "jpx",
            Self::Crypt => "crypt",
        }
    }

    pub(crate) fn from_name(name: Name<'_>) -> Option<Self> {
        match name.deref() {
            ASCII_HEX_DECODE | ASCII_HEX_DECODE_ABBREVIATION => Some(Self::AsciiHexDecode),
            ASCII85_DECODE | ASCII85_DECODE_ABBREVIATION => Some(Self::Ascii85Decode),
            LZW_DECODE | LZW_DECODE_ABBREVIATION => Some(Self::LzwDecode),
            FLATE_DECODE | FLATE_DECODE_ABBREVIATION => Some(Self::FlateDecode),
            RUN_LENGTH_DECODE | RUN_LENGTH_DECODE_ABBREVIATION => Some(Self::RunLengthDecode),
            CCITTFAX_DECODE | CCITTFAX_DECODE_ABBREVIATION => Some(Self::CcittFaxDecode),
            JBIG2_DECODE => Some(Self::Jbig2Decode),
            DCT_DECODE | DCT_DECODE_ABBREVIATION => Some(Self::DctDecode),
            JPX_DECODE => Some(Self::JpxDecode),
            CRYPT => Some(Self::Crypt),
            _ => {
                warn!("unknown filter: {}", name.as_str());

                None
            }
        }
    }

    pub(crate) fn apply(
        &self,
        data: &[u8],
        params: &Dict<'_>,
        #[cfg_attr(not(feature = "images"), allow(unused))] image_params: &ImageDecodeParams,
        #[cfg_attr(not(feature = "images"), allow(unused))] stop: &Arc<dyn Stop>,
    ) -> Result<FilterResult<'static>, DecodeFailure> {
        // Each decoder reports a `DecodeFailure` directly, so a stop
        // (`DecodeFailure::Stopped`) is distinguished from a genuine decode
        // failure without re-polling the stop after the fact.
        let res = match self {
            Self::AsciiHexDecode => ascii_hex::decode(data)
                .map(FilterResult::from_data)
                .ok_or(DecodeFailure::StreamDecode),
            Self::Ascii85Decode => ascii_85::decode(data)
                .map(FilterResult::from_data)
                .ok_or(DecodeFailure::StreamDecode),
            Self::RunLengthDecode => {
                run_length::decode(data, stop.as_ref()).map(FilterResult::from_data)
            }
            Self::LzwDecode => {
                lzw_flate::lzw::decode(data, params, stop.as_ref()).map(FilterResult::from_data)
            }
            Self::FlateDecode => {
                lzw_flate::flate::decode(data, params, stop.as_ref()).map(FilterResult::from_data)
            }
            #[cfg(feature = "images")]
            Self::DctDecode => dct::decode(data, params, image_params, stop),
            #[cfg(feature = "images")]
            Self::CcittFaxDecode => ccitt::decode(data, params, image_params, stop.as_ref()),
            #[cfg(feature = "images")]
            Self::Jbig2Decode => jbig2::decode(data, params, image_params, stop.as_ref()),
            #[cfg(feature = "images")]
            Self::JpxDecode => jpx::decode(data, image_params, stop.as_ref()),
            #[cfg(not(feature = "images"))]
            Self::DctDecode | Self::CcittFaxDecode | Self::Jbig2Decode | Self::JpxDecode => {
                warn!("image decoding is not supported (enable the `images` feature)");
                Err(DecodeFailure::ImageDecode)
            }
            _ => Err(DecodeFailure::StreamDecode),
        };

        // A stop is an expected outcome; only a genuine failure warrants a warning.
        if matches!(&res, Err(e) if !matches!(e, DecodeFailure::Stopped)) {
            warn!("failed to apply filter {}", self.debug_name());
        }

        res
    }
}
