/// An error that can occur during CCITT decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// Unexpected end of input while reading bits.
    UnexpectedEof,
    /// Invalid Huffman code sequence was encountered during decoding.
    InvalidCode,
    /// Arithmetic overflow in run length or position calculation.
    Overflow,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of input"),
            Self::InvalidCode => write!(f, "invalid CCITT code sequence"),
            Self::Overflow => write!(f, "arithmetic overflow in position calculation"),
        }
    }
}

impl core::error::Error for DecodeError {}

/// A specialized Result type for CCITT decoding operations.
pub type Result<T> = core::result::Result<T, DecodeError>;
