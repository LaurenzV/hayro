//! Error types for the PostScript scanner.

use core::fmt;

/// A specialized [`Result`] type for PostScript scanner operations.
pub type Result<T> = core::result::Result<T, Error>;

/// An error encountered while scanning a PostScript token stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A syntax error in the input (e.g. malformed string, unexpected delimiter).
    SyntaxError,
    /// A numeric value exceeded implementation limits.
    LimitCheck,
    /// An unsupported PostScript type was encountered (e.g. `<<` dictionary,
    /// `{` procedure).
    UnsupportedType,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::SyntaxError => f.write_str("syntaxerror"),
            Error::LimitCheck => f.write_str("limitcheck"),
            Error::UnsupportedType => f.write_str("unsupported type"),
        }
    }
}
