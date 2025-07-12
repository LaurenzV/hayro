//! Decoding data streams.

mod ascii_85;
pub(crate) mod ascii_hex;
mod ccitt;
mod dct;
mod jbig2;
mod jpx;
mod lzw_flate;
mod run_length;

use crate::object::dict::Dict;
use crate::object::dict::keys::*;
use crate::object::name::Name;
use crate::util::OptionLog;
use log::warn;
use std::ops::Deref;

/// A filter.
#[derive(Debug, Copy, Clone)]
pub enum Filter {
    /// The ASCII-hex filter.
    AsciiHexDecode,
    /// The ASCII85 filter.
    Ascii85Decode,
    /// The LZW filter.
    LzwDecode,
    /// The flate (zlib/deflate) filter.
    FlateDecode,
    /// The run-length filter.
    RunLengthDecode,
    /// The CCITT Fax filter.
    CcittFaxDecode,
    /// The JBIG2 filter.
    Jbig2Decode,
    /// The DCT (JPEG) filter.
    DctDecode,
    /// The JPX (JPEG 2000) filter.
    JpxDecode,
    /// The crypt filter.
    Crypt,
}

/// An image color space.
pub enum ImageColorSpace {
    /// Grayscale color space.
    Gray,
    /// RGB color space.
    Rgb,
    /// CMYK color space.
    Cmyk,
}

impl ImageColorSpace {
    fn num_components(&self) -> u8 {
        match self {
            ImageColorSpace::Gray => 1,
            ImageColorSpace::Rgb => 3,
            ImageColorSpace::Cmyk => 4,
        }
    }
}

/// The result of the filter.
pub struct FilterResult {
    /// The decoded data.
    pub data: Vec<u8>,
    /// An optional alpha channel of the image (will only be set for JPX streams).
    pub alpha: Option<Vec<u8>>,
    /// The color space of the image (will only be set for JPX streams).
    pub color_space: Option<ImageColorSpace>,
    /// The bits per component of the image (will only be set for JPX streams).
    pub bits_per_component: Option<u8>,
}

impl FilterResult {
    fn from_data(data: Vec<u8>) -> Self {
        Self {
            data,
            alpha: None,
            color_space: None,
            bits_per_component: None,
        }
    }
}

impl Filter {
    fn debug_name(&self) -> &'static str {
        match self {
            Filter::AsciiHexDecode => "ascii_hex",
            Filter::Ascii85Decode => "ascii_85",
            Filter::LzwDecode => "lzw",
            Filter::FlateDecode => "flate",
            Filter::RunLengthDecode => "run-length",
            Filter::CcittFaxDecode => "ccit_fax",
            Filter::Jbig2Decode => "jbig2",
            Filter::DctDecode => "dct",
            Filter::JpxDecode => "jpx",
            Filter::Crypt => "crypt",
        }
    }

    pub(crate) fn from_name(name: Name) -> Option<Self> {
        match name.deref() {
            ASCII_HEX_DECODE | ASCII_HEX_DECODE_ABBREVIATION => Some(Filter::AsciiHexDecode),
            ASCII85_DECODE | ASCII85_DECODE_ABBREVIATION => Some(Filter::Ascii85Decode),
            LZW_DECODE | LZW_DECODE_ABBREVIATION => Some(Filter::LzwDecode),
            FLATE_DECODE | FLATE_DECODE_ABBREVIATION => Some(Filter::FlateDecode),
            RUN_LENGTH_DECODE | RUN_LENGTH_DECODE_ABBREVIATION => Some(Filter::RunLengthDecode),
            CCITTFAX_DECODE | CCITTFAX_DECODE_ABBREVIATION => Some(Filter::CcittFaxDecode),
            JBIG2_DECODE => Some(Filter::Jbig2Decode),
            DCT_DECODE | DCT_DECODE_ABBREVIATION => Some(Filter::DctDecode),
            JPX_DECODE => Some(Filter::JpxDecode),
            CRYPT => Some(Filter::Crypt),
            _ => {
                warn!("unknown filter: {}", name.as_str());

                None
            }
        }
    }

    /// Apply the filter to some data.
    pub fn apply(&self, data: &[u8], params: Dict) -> Option<FilterResult> {
        match self {
            Filter::AsciiHexDecode => ascii_hex::decode(data).map(FilterResult::from_data),
            Filter::Ascii85Decode => ascii_85::decode(data).map(FilterResult::from_data),
            Filter::RunLengthDecode => run_length::decode(data).map(FilterResult::from_data),
            Filter::LzwDecode => lzw_flate::lzw::decode(data, params).map(FilterResult::from_data),
            Filter::DctDecode => dct::decode(data, params).map(FilterResult::from_data),
            Filter::FlateDecode => {
                lzw_flate::flate::decode(data, params).map(FilterResult::from_data)
            }
            Filter::CcittFaxDecode => ccitt::decode(data, params).map(FilterResult::from_data),
            Filter::Jbig2Decode => Some(FilterResult::from_data(jbig2::decode(data, params)?)),
            Filter::JpxDecode => jpx::decode(data),
            _ => None,
        }
        .error_none(&format!("failed to apply filter {}", self.debug_name()))
    }
}
