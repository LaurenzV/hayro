//! Object enum and top-level read dispatch.

use core::fmt;
use core::ops::Deref;

use smallvec::SmallVec;

use crate::reader::Reader;
use crate::{hex_string, name, number, operator, string};

/// Inline byte buffer used for all string-like object payloads.
#[derive(Clone, Eq)]
pub struct Bytes(SmallVec<[u8; 8]>);

impl Bytes {
    pub(crate) fn new() -> Self {
        Self(SmallVec::new())
    }

    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self(SmallVec::with_capacity(cap))
    }

    pub(crate) fn push(&mut self, b: u8) {
        self.0.push(b);
    }

    pub fn from_slice(s: &[u8]) -> Self {
        Self(SmallVec::from_slice(s))
    }
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl PartialEq<[u8]> for Bytes {
    fn eq(&self, other: &[u8]) -> bool {
        **self == *other
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for Bytes {
    fn eq(&self, other: &&[u8; N]) -> bool {
        **self == **other
    }
}

impl<const N: usize> PartialEq<[u8; N]> for Bytes {
    fn eq(&self, other: &[u8; N]) -> bool {
        **self == *other
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b\"")?;
        for &byte in &self.0 {
            match byte {
                b' '..=b'~' => write!(f, "{}", byte as char)?,
                _ => write!(f, "\\x{byte:02X}")?,
            }
        }
        write!(f, "\"")
    }
}

/// Shorthand for [`Bytes::from_slice`].
///
/// ```
/// # use hayro_postscript::bytes;
/// let b = bytes!(b"Hello");
/// assert_eq!(&*b, b"Hello");
/// ```
#[macro_export]
macro_rules! bytes {
    ($s:expr) => {
        $crate::Bytes::from_slice($s)
    };
}

/// A PostScript object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Object {
    /// An integer value.
    Integer(i32),
    /// A name object (e.g. `/CMapName`). The bytes do not include the leading `/`.
    Name(Bytes),
    /// A literal string (e.g. `(Hello)`).
    String(Bytes),
    /// A hex string (e.g. `<48656C6C6F>`).
    HexString(Bytes),
    /// A bare-word operator or single-character delimiter used as an operator
    /// (e.g. `beginbfchar`, `def`, `[`, `]`, `{`, `}`, `<<`, `>>`).
    Operator(Bytes),
}

/// Try to read the next PostScript object from the reader.
///
/// Whitespace and comments are skipped before dispatching.
/// Returns `None` at EOF.
pub(crate) fn read(r: &mut Reader<'_>) -> Option<Object> {
    skip_whitespace_and_comments(r);

    let b = r.peek_byte()?;

    match b {
        b'(' => string::read(r).map(Object::String),
        b'<' => {
            // `<<` is a dict-open operator.
            if r.peek_bytes(2) == Some(b"<<") {
                r.forward();
                r.forward();
                Some(Object::Operator(Bytes::from_slice(b"<<")))
            } else {
                hex_string::read(r).map(Object::HexString)
            }
        }
        b'>' => {
            // `>>` is a dict-close operator.
            if r.peek_bytes(2) == Some(b">>") {
                r.forward();
                r.forward();
                Some(Object::Operator(Bytes::from_slice(b">>")))
            } else {
                // Stray `>` — shouldn't happen in valid PS, but consume it.
                r.forward();
                Some(Object::Operator(Bytes::from_slice(b">")))
            }
        }
        b'/' => name::read(r).map(Object::Name),
        b'[' | b']' | b'{' | b'}' => {
            r.forward();
            Some(Object::Operator(Bytes::from_slice(&[b])))
        }
        b'+' | b'-' | b'0'..=b'9' => {
            // Try integer first; fall through to operator if it doesn't parse.
            if let Some(n) = number::read(r) {
                Some(Object::Integer(n))
            } else {
                operator::read(r).map(Object::Operator)
            }
        }
        _ => operator::read(r).map(Object::Operator),
    }
}

fn skip_whitespace_and_comments(r: &mut Reader<'_>) {
    loop {
        match r.peek_byte() {
            Some(b) if crate::reader::is_whitespace(b) => {
                r.forward();
            }
            Some(b'%') => {
                // Comment: skip to end of line.
                r.forward();
                r.forward_while(|b| !crate::reader::is_eol(b));
            }
            _ => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_one(input: &[u8]) -> Option<Object> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    #[test]
    fn integer() {
        assert_eq!(read_one(b"42 "), Some(Object::Integer(42)));
    }

    #[test]
    fn negative_integer() {
        assert_eq!(read_one(b"-7 "), Some(Object::Integer(-7)));
    }

    #[test]
    fn name_simple() {
        assert_eq!(
            read_one(b"/CMapName "),
            Some(Object::Name(bytes!(b"CMapName")))
        );
    }

    #[test]
    fn literal_string() {
        assert_eq!(
            read_one(b"(Hello)"),
            Some(Object::String(bytes!(b"Hello")))
        );
    }

    #[test]
    fn hex_string() {
        assert_eq!(
            read_one(b"<48656C6C6F>"),
            Some(Object::HexString(bytes!(b"Hello")))
        );
    }

    #[test]
    fn operator_word() {
        assert_eq!(
            read_one(b"beginbfchar "),
            Some(Object::Operator(bytes!(b"beginbfchar")))
        );
    }

    #[test]
    fn dict_open() {
        assert_eq!(
            read_one(b"<< "),
            Some(Object::Operator(bytes!(b"<<")))
        );
    }

    #[test]
    fn dict_close() {
        assert_eq!(
            read_one(b">> "),
            Some(Object::Operator(bytes!(b">>")))
        );
    }

    #[test]
    fn bracket_operators() {
        assert_eq!(
            read_one(b"["),
            Some(Object::Operator(bytes!(b"[")))
        );
        assert_eq!(
            read_one(b"]"),
            Some(Object::Operator(bytes!(b"]")))
        );
        assert_eq!(
            read_one(b"{"),
            Some(Object::Operator(bytes!(b"{")))
        );
        assert_eq!(
            read_one(b"}"),
            Some(Object::Operator(bytes!(b"}")))
        );
    }

    #[test]
    fn skips_whitespace() {
        assert_eq!(read_one(b"  \t\n 42 "), Some(Object::Integer(42)));
    }

    #[test]
    fn skips_comments() {
        assert_eq!(
            read_one(b"% this is a comment\n42 "),
            Some(Object::Integer(42))
        );
    }

    #[test]
    fn eof() {
        assert_eq!(read_one(b""), None);
        assert_eq!(read_one(b"   "), None);
        assert_eq!(read_one(b"% comment only\n"), None);
    }
}
