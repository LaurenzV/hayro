mod ascii_85;
pub(crate) mod ascii_hex;
mod literal;

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::reader::Reader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringInner<'a> {
    Literal(&'a [u8]),
    Hex(&'a [u8]),
    Ascii85(&'a [u8]),
}

/// A PostScript string object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct String<'a> {
    inner: StringInner<'a>,
}

impl<'a> String<'a> {
    pub(crate) const fn from_literal(data: &'a [u8]) -> Self {
        Self {
            inner: StringInner::Literal(data),
        }
    }

    pub(crate) const fn from_hex(data: &'a [u8]) -> Self {
        Self {
            inner: StringInner::Hex(data),
        }
    }

    pub(crate) const fn from_ascii85(data: &'a [u8]) -> Self {
        Self {
            inner: StringInner::Ascii85(data),
        }
    }

    /// Decode the string content into `out`, replacing any previous contents.
    pub fn decode_into(&self, out: &mut Vec<u8>) -> Result<()> {
        out.clear();
        match self.inner {
            StringInner::Literal(data) => literal::decode_into(data, out),
            StringInner::Hex(data) => ascii_hex::decode_into(data, out),
            StringInner::Ascii85(data) => ascii_85::decode_into(data, out),
        }
        .ok_or(Error::SyntaxError)
    }

    /// Decode the string content.
    pub fn decode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.decode_into(&mut out)?;
        Ok(out)
    }
}

pub(crate) fn parse_literal<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    let start = r.offset();
    skip_literal(r)?;
    let end = r.offset();
    // Exclude outer parentheses.
    r.range(start + 1..end - 1)
}

pub(crate) fn parse_hex<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    r.forward_tag(b"<")?;
    let start = r.offset();
    while let Some(b) = r.read_byte() {
        if b == b'>' {
            return r.range(start..r.offset() - 1);
        }
    }
    None
}

pub(crate) fn parse_ascii85<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    r.forward_tag(b"<~")?;
    let start = r.offset();
    loop {
        let b = r.read_byte()?;
        if b == b'~' {
            let end = r.offset() - 1;
            r.forward_tag(b">")?;
            return r.range(start..end);
        }
    }
}

fn skip_literal(r: &mut Reader<'_>) -> Option<()> {
    r.forward_tag(b"(")?;
    let mut depth = 1_u32;

    while depth > 0 {
        let byte = r.read_byte()?;
        match byte {
            b'\\' => {
                let _ = r.read_byte()?;
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_literal(input: &[u8]) -> Result<Vec<u8>> {
        let mut r = Reader::new(input);
        let data = parse_literal(&mut r).ok_or(Error::SyntaxError)?;
        String::from_literal(data).decode()
    }

    #[test]
    fn literal_empty() {
        assert_eq!(decode_literal(b"()").unwrap(), b"");
    }

    #[test]
    fn literal_simple() {
        assert_eq!(decode_literal(b"(Hello)").unwrap(), b"Hello");
    }

    #[test]
    fn literal_nested_parens() {
        assert_eq!(
            decode_literal(b"(Hi (()) there)").unwrap(),
            b"Hi (()) there"
        );
    }

    #[test]
    fn literal_escape_n() {
        assert_eq!(decode_literal(b"(a\\nb)").unwrap(), b"a\nb");
    }

    #[test]
    fn literal_escape_r() {
        assert_eq!(decode_literal(b"(a\\rb)").unwrap(), b"a\rb");
    }

    #[test]
    fn literal_escape_t() {
        assert_eq!(decode_literal(b"(a\\tb)").unwrap(), b"a\tb");
    }

    #[test]
    fn literal_escape_b() {
        assert_eq!(decode_literal(b"(a\\bb)").unwrap(), &[b'a', 0x08, b'b']);
    }

    #[test]
    fn literal_escape_f() {
        assert_eq!(decode_literal(b"(a\\fb)").unwrap(), &[b'a', 0x0C, b'b']);
    }

    #[test]
    fn literal_escape_backslash() {
        assert_eq!(decode_literal(b"(a\\\\b)").unwrap(), b"a\\b");
    }

    #[test]
    fn literal_escape_parens() {
        assert_eq!(decode_literal(b"(Hi \\()").unwrap(), b"Hi (");
    }

    #[test]
    fn literal_octal_three_digits() {
        assert_eq!(decode_literal(b"(\\053)").unwrap(), b"+");
    }

    #[test]
    fn literal_octal_two_digits() {
        assert_eq!(decode_literal(b"(\\36)").unwrap(), b"\x1e");
    }

    #[test]
    fn literal_octal_one_digit() {
        assert_eq!(decode_literal(b"(\\3)").unwrap(), b"\x03");
    }

    #[test]
    fn literal_line_continuation_lf() {
        assert_eq!(decode_literal(b"(Hi \\\nthere)").unwrap(), b"Hi there");
    }

    #[test]
    fn literal_line_continuation_cr() {
        assert_eq!(decode_literal(b"(Hi \\\rthere)").unwrap(), b"Hi there");
    }

    #[test]
    fn literal_line_continuation_crlf() {
        assert_eq!(decode_literal(b"(Hi \\\r\nthere)").unwrap(), b"Hi there");
    }

    #[test]
    fn literal_bare_eol_lf() {
        assert_eq!(decode_literal(b"(a\nb)").unwrap(), b"a\nb");
    }

    #[test]
    fn literal_bare_eol_cr() {
        assert_eq!(decode_literal(b"(a\rb)").unwrap(), b"a\nb");
    }

    #[test]
    fn literal_bare_eol_crlf() {
        assert_eq!(decode_literal(b"(a\r\nb)").unwrap(), b"a\nb");
    }

    fn decode_hex(input: &[u8]) -> Result<Vec<u8>> {
        let mut r = Reader::new(input);
        let data = parse_hex(&mut r).ok_or(Error::SyntaxError)?;
        String::from_hex(data).decode()
    }

    #[test]
    fn hex_simple() {
        assert_eq!(decode_hex(b"<48656C6C6F>").unwrap(), b"Hello");
    }

    #[test]
    fn hex_with_whitespace() {
        assert_eq!(decode_hex(b"<48 65 6C 6C 6F>").unwrap(), b"Hello");
    }

    #[test]
    fn hex_odd_nibble() {
        assert_eq!(decode_hex(b"<ABC>").unwrap(), &[0xAB, 0xC0]);
    }

    #[test]
    fn hex_empty() {
        assert_eq!(decode_hex(b"<>").unwrap(), b"");
    }

    #[test]
    fn hex_lowercase() {
        assert_eq!(decode_hex(b"<abcd>").unwrap(), &[0xAB, 0xCD]);
    }

    #[test]
    fn hex_mixed_case() {
        assert_eq!(decode_hex(b"<aB3E>").unwrap(), &[0xAB, 0x3E]);
    }

    fn decode_a85(input: &[u8]) -> Result<Vec<u8>> {
        let mut r = Reader::new(input);
        let data = parse_ascii85(&mut r).ok_or(Error::SyntaxError)?;
        String::from_ascii85(data).decode()
    }

    #[test]
    fn ascii85_simple() {
        // "Hello" in ASCII85 is "87cURDZ"
        assert_eq!(decode_a85(b"<~87cURDZ~>").unwrap(), b"Hello");
    }

    #[test]
    fn ascii85_empty() {
        assert_eq!(decode_a85(b"<~~>").unwrap(), b"");
    }

    #[test]
    fn ascii85_z_shorthand() {
        assert_eq!(decode_a85(b"<~z~>").unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn ascii85_partial_group() {
        let result = decode_a85(b"<~87~>").unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn ascii85_with_whitespace() {
        assert_eq!(decode_a85(b"<~87cU RDZ~>").unwrap(), b"Hello");
    }
}
