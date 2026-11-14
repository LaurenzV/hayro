/// An error that can occur during CCITT decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Unexpected end of input while reading bits.
    UnexpectedEof,
    /// An invalid code was encountered.
    InvalidCode,
    /// Arithmetic overflow in run length or position calculation.
    Overflow,
    /// Decoding was cancelled.
    Cancelled,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of input"),
            Self::InvalidCode => write!(f, "invalid CCITT code sequence"),
            Self::Overflow => write!(f, "arithmetic overflow in position calculation"),
            Self::Cancelled => write!(f, "decoding was cancelled"),
        }
    }
}

impl core::error::Error for DecodeError {}

/// The result type of decoding.
pub type Result<T> = core::result::Result<T, DecodeError>;
