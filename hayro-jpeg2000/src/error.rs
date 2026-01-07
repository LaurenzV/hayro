//! Error types for JPEG 2000 decoding.

use core::fmt;

/// The main error type for JPEG 2000 decoding operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Errors related to JP2 file format and box parsing.
    Format(FormatError),
    /// Errors related to codestream markers.
    Marker(MarkerError),
    /// Errors related to tile processing.
    Tile(TileError),
    /// Errors related to image dimensions and validation.
    Validation(ValidationError),
    /// Errors related to decoding operations.
    Decoding(DecodingError),
    /// Errors related to color space and component handling.
    Color(ColorError),
}

/// Errors related to JP2 file format and box parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// Invalid JP2 signature.
    InvalidSignature,
    /// Invalid JP2 file type.
    InvalidFileType,
    /// Invalid or malformed JP2 box.
    InvalidBox,
    /// Missing codestream data.
    MissingCodestream,
    /// Unsupported JP2 image format.
    Unsupported,
}

/// Errors related to codestream markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerError {
    /// Invalid marker encountered.
    Invalid,
    /// Unsupported marker encountered.
    Unsupported,
    /// Expected a specific marker.
    Expected(&'static str),
    /// Missing a required marker.
    Missing(&'static str),
    /// Failed to read or parse a marker.
    ParseFailure(&'static str),
}

/// Errors related to tile processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileError {
    /// Invalid image tile was encountered.
    Invalid,
    /// Invalid tile index in tile-part header.
    InvalidIndex,
    /// Invalid tile or image offsets.
    InvalidOffsets,
    /// PPT marker present when PPM marker exists in main header.
    PpmPptConflict,
}

/// Errors related to image dimensions and validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    /// Invalid image dimensions.
    InvalidDimensions,
    /// Image dimensions exceed supported limits.
    ImageTooLarge,
    /// Image has too many channels.
    TooManyChannels,
    /// Invalid component metadata.
    InvalidComponentMetadata,
    /// Invalid progression order.
    InvalidProgressionOrder,
    /// Invalid transformation type.
    InvalidTransformation,
    /// Invalid quantization style.
    InvalidQuantizationStyle,
    /// Missing exponents for precinct sizes.
    MissingPrecinctExponents,
    /// Not enough exponents provided in header.
    InsufficientExponents,
    /// Missing exponent step size.
    MissingStepSize,
    /// Invalid quantization exponents.
    InvalidExponents,
}

/// Errors related to decoding operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodingError {
    /// An error occurred while decoding a code-block.
    CodeBlockDecodeFailure,
    /// Number of bitplanes in a codeblock is too large.
    TooManyBitplanes,
    /// A codeblock contains too many coding passes.
    TooManyCodingPasses,
    /// Invalid number of bitplanes in a codeblock.
    InvalidBitplaneCount,
    /// A precinct was invalid.
    InvalidPrecinct,
    /// A progression iterator ver invalid.
    InvalidProgressionIterator,
    /// Unexpected end of data.
    UnexpectedEof,
}

/// Errors related to color space and component handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorError {
    /// Multi-component transform failed.
    Mct,
    /// Failed to resolve palette indices.
    PaletteResolutionFailed,
    /// Failed to convert from sYCC to RGB.
    SyccConversionFailed,
    /// Failed to convert from LAB to RGB.
    LabConversionFailed,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Format(e) => write!(f, "{e}"),
            DecodeError::Marker(e) => write!(f, "{e}"),
            DecodeError::Tile(e) => write!(f, "{e}"),
            DecodeError::Validation(e) => write!(f, "{e}"),
            DecodeError::Decoding(e) => write!(f, "{e}"),
            DecodeError::Color(e) => write!(f, "{e}"),
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::InvalidSignature => write!(f, "invalid JP2 signature"),
            FormatError::InvalidFileType => write!(f, "invalid JP2 file type"),
            FormatError::InvalidBox => write!(f, "invalid JP2 box"),
            FormatError::MissingCodestream => write!(f, "missing codestream data"),
            FormatError::Unsupported => write!(f, "unsupported JP2 image"),
        }
    }
}

impl fmt::Display for MarkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarkerError::Invalid => write!(f, "invalid marker"),
            MarkerError::Unsupported => write!(f, "unsupported marker"),
            MarkerError::Expected(marker) => write!(f, "expected {marker} marker"),
            MarkerError::Missing(marker) => write!(f, "missing {marker} marker"),
            MarkerError::ParseFailure(marker) => write!(f, "failed to read {marker} marker"),
        }
    }
}

impl fmt::Display for TileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TileError::Invalid => write!(f, "image contains no tiles"),
            TileError::InvalidIndex => write!(f, "invalid tile index in tile-part header"),
            TileError::InvalidOffsets => write!(f, "invalid tile offsets"),
            TileError::PpmPptConflict => {
                write!(
                    f,
                    "PPT marker present when PPM marker exists in main header"
                )
            }
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::InvalidDimensions => write!(f, "invalid image dimensions"),
            ValidationError::ImageTooLarge => write!(f, "image is too large"),
            ValidationError::TooManyChannels => write!(f, "image has too many channels"),
            ValidationError::InvalidComponentMetadata => write!(f, "invalid component metadata"),
            ValidationError::InvalidProgressionOrder => write!(f, "invalid progression order"),
            ValidationError::InvalidTransformation => write!(f, "invalid transformation type"),
            ValidationError::InvalidQuantizationStyle => write!(f, "invalid quantization style"),
            ValidationError::MissingPrecinctExponents => {
                write!(f, "missing exponents for precinct sizes")
            }
            ValidationError::InsufficientExponents => {
                write!(f, "not enough exponents provided in header")
            }
            ValidationError::MissingStepSize => write!(f, "missing exponent step size"),
            ValidationError::InvalidExponents => write!(f, "invalid quantization exponents"),
        }
    }
}

impl fmt::Display for DecodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodingError::CodeBlockDecodeFailure => write!(f, "failed to decode code-block"),
            DecodingError::TooManyBitplanes => write!(f, "number of bitplanes is too large"),
            DecodingError::TooManyCodingPasses => {
                write!(f, "codeblock contains too many coding passes")
            }
            DecodingError::InvalidBitplaneCount => write!(f, "invalid number of bitplanes"),
            DecodingError::InvalidPrecinct => write!(f, "failed to build precincts"),
            DecodingError::InvalidProgressionIterator => {
                write!(f, "failed to build progression iterator")
            }
            DecodingError::UnexpectedEof => write!(f, "unexpected end of data"),
        }
    }
}

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorError::Mct => write!(f, "multi-component transform failed"),
            ColorError::PaletteResolutionFailed => write!(f, "failed to resolve palette indices"),
            ColorError::SyccConversionFailed => write!(f, "failed to convert from sYCC to RGB"),
            ColorError::LabConversionFailed => write!(f, "failed to convert from LAB to RGB"),
        }
    }
}

impl std::error::Error for DecodeError {}
impl std::error::Error for FormatError {}
impl std::error::Error for MarkerError {}
impl std::error::Error for TileError {}
impl std::error::Error for ValidationError {}
impl std::error::Error for DecodingError {}
impl std::error::Error for ColorError {}

impl From<FormatError> for DecodeError {
    fn from(e: FormatError) -> Self {
        DecodeError::Format(e)
    }
}

impl From<MarkerError> for DecodeError {
    fn from(e: MarkerError) -> Self {
        DecodeError::Marker(e)
    }
}

impl From<TileError> for DecodeError {
    fn from(e: TileError) -> Self {
        DecodeError::Tile(e)
    }
}

impl From<ValidationError> for DecodeError {
    fn from(e: ValidationError) -> Self {
        DecodeError::Validation(e)
    }
}

impl From<DecodingError> for DecodeError {
    fn from(e: DecodingError) -> Self {
        DecodeError::Decoding(e)
    }
}

impl From<ColorError> for DecodeError {
    fn from(e: ColorError) -> Self {
        DecodeError::Color(e)
    }
}

/// Result type for JPEG 2000 decoding operations.
pub type Result<T> = core::result::Result<T, DecodeError>;
