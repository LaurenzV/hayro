//! Name object parsing.
//!
//! A name stores its raw bytes lazily. Call [`Name::decode`] to resolve
//! `#XX` hex escapes.

use alloc::vec::Vec;

use crate::reader::{Reader, is_regular};
use crate::filter::ascii_hex::decode_hex_digit;

/// A PostScript name object.
///
/// Names are stored as raw bytes without decoding `#XX` hex escapes.
/// Use [`decode`](Name::decode) to materialise the decoded form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Name<'a> {
    data: &'a [u8],
    literal: bool,
}

impl<'a> Name<'a> {
    pub(crate) fn new(data: &'a [u8], literal: bool) -> Self {
        Self { data, literal }
    }

    /// The raw bytes of the name (without the leading `/` for literal names,
    /// and without `#XX` decoding).
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Returns `true` if this is a literal name (introduced by `/`).
    ///
    /// Executable names (bare words) return `false`.
    pub fn is_literal(&self) -> bool {
        self.literal
    }

    /// Decode `#XX` hex escapes and return the result.
    ///
    /// Returns `None` if a `#XX` escape is malformed.
    pub fn decode(&self) -> Option<Vec<u8>> {
        // Fast path: no `#` escapes.
        if !self.data.contains(&b'#') {
            return Some(self.data.to_vec());
        }

        // Slow path: decode `#XX` hex escapes.
        let mut result = Vec::with_capacity(self.data.len());
        let mut inner = Reader::new(self.data);

        while let Some(b) = inner.read_byte() {
            if b == b'#' {
                let hex = inner.read_bytes(2)?;
                result.push(decode_hex_digit(hex[0])? << 4 | decode_hex_digit(hex[1])?);
            } else {
                result.push(b);
            }
        }

        Some(result)
    }
}

/// Parse a literal name `/...`, returning the raw bytes after the `/`.
///
/// The reader must be positioned at the leading `/`.
pub(crate) fn parse_literal<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    r.forward_tag(b"/")?;
    let start = r.offset();
    while r.eat(is_regular).is_some() {}
    r.range(start..r.offset())
}

/// Parse an executable name (bare word of regular characters).
///
/// The reader must be positioned at the first character.
pub(crate) fn parse_executable<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    let start = r.offset();
    r.forward_while(is_regular);
    if r.offset() == start {
        return None;
    }
    r.range(start..r.offset())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_literal(input: &[u8]) -> Option<Name<'_>> {
        let mut r = Reader::new(input);
        parse_literal(&mut r).map(|d| Name::new(d, true))
    }

    fn read_executable(input: &[u8]) -> Option<Name<'_>> {
        let mut r = Reader::new(input);
        parse_executable(&mut r).map(|d| Name::new(d, false))
    }

    #[test]
    fn literal_simple() {
        let n = read_literal(b"/Name1").unwrap();
        assert_eq!(n.data(), b"Name1");
        assert!(n.is_literal());
    }

    #[test]
    fn literal_empty_name() {
        let n = read_literal(b"/").unwrap();
        assert_eq!(n.data(), b"");
        assert!(n.is_literal());
    }

    #[test]
    fn literal_with_hex_escape() {
        let n = read_literal(b"/lime#20Green").unwrap();
        assert_eq!(n.data(), b"lime#20Green");
        assert_eq!(n.decode().unwrap(), b"lime Green");
    }

    #[test]
    fn literal_multiple_hex_escapes() {
        let n = read_literal(b"/paired#28#29parentheses").unwrap();
        assert_eq!(n.decode().unwrap(), b"paired()parentheses");
    }

    #[test]
    fn literal_special_chars() {
        let n = read_literal(b"/A;Name_With-Various***Characters?").unwrap();
        assert_eq!(n.data(), b"A;Name_With-Various***Characters?");
    }

    #[test]
    fn literal_stops_at_delimiter() {
        let mut r = Reader::new(b"/Name(rest");
        let data = parse_literal(&mut r).unwrap();
        assert_eq!(data, b"Name");
        assert_eq!(r.peek_byte(), Some(b'('));
    }

    #[test]
    fn literal_stops_at_whitespace() {
        let mut r = Reader::new(b"/Name rest");
        let data = parse_literal(&mut r).unwrap();
        assert_eq!(data, b"Name");
        assert_eq!(r.peek_byte(), Some(b' '));
    }

    #[test]
    fn literal_not_a_name() {
        assert!(read_literal(b"Name").is_none());
    }

    #[test]
    fn executable_simple() {
        let n = read_executable(b"beginbfchar ").unwrap();
        assert_eq!(n.data(), b"beginbfchar");
        assert!(!n.is_literal());
    }

    #[test]
    fn executable_stops_at_delimiter() {
        let mut r = Reader::new(b"def/name");
        let data = parse_executable(&mut r).unwrap();
        assert_eq!(data, b"def");
        assert_eq!(r.peek_byte(), Some(b'/'));
    }

    #[test]
    fn executable_at_eof() {
        let n = read_executable(b"endcmap").unwrap();
        assert_eq!(n.data(), b"endcmap");
    }

    #[test]
    fn executable_empty() {
        assert!(read_executable(b"").is_none());
    }

    #[test]
    fn executable_starts_at_delimiter() {
        assert!(read_executable(b"(foo)").is_none());
    }

    #[test]
    fn decode_no_escapes() {
        let n = Name::new(b"simple", true);
        assert_eq!(n.decode().unwrap(), b"simple");
    }

    #[test]
    fn decode_with_escapes() {
        let n = Name::new(b"lime#20Green", true);
        assert_eq!(n.decode().unwrap(), b"lime Green");
    }
}
