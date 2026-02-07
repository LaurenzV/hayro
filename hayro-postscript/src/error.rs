//! Error types for the PostScript scanner.

use core::fmt;

/// A specialized [`Result`] type for PostScript scanner operations.
pub type Result<T> = core::result::Result<T, Error>;

/// An error encountered while scanning a PostScript token stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A syntax error in the input.
    SyntaxError,
    /// A numeric value exceeded implementation limits.
    LimitCheck,
    /// An unsupported PostScript type was encountered (like dictionaries or
    /// procedures, which will be added in the future).
    UnsupportedType,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SyntaxError => f.write_str("syntaxerror"),
            Self::LimitCheck => f.write_str("limitcheck"),
            Self::UnsupportedType => f.write_str("unsupported type"),
        }
    }
}

impl core::error::Error for Error {}
