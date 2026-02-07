//! Array `[...]` parsing.
//!
//! The parser only finds the extent of the array (the matching `]`).
//! The inner objects are lazily produced via [`Array::objects`].

use crate::error::Error;
use crate::reader::Reader;

/// A PostScript array object.
///
/// Stores a reference to the raw bytes between `[` and `]`, without parsing
/// the inner objects. Call [`objects`](Array::objects) to iterate over the
/// contained objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Array<'a> {
    data: &'a [u8],
}

impl<'a> Array<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// The raw bytes between `[` and `]`.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Return a [`Scanner`](crate::Scanner) that iterates over the objects inside
    /// this array.
    pub fn objects(&self) -> crate::Scanner<'a> {
        crate::Scanner::new(self.data)
    }
}

/// Parse an array `[...]`, returning the raw inner bytes.
///
/// The reader must be positioned at the opening `[`.
pub(crate) fn parse<'a>(r: &mut Reader<'a>) -> Result<&'a [u8], Error> {
    r.forward_tag(b"[").ok_or(Error::SyntaxError)?;
    let start = r.offset();
    skip_to_matching_bracket(r)?;
    let end = r.offset() - 1; // exclude the closing `]`
    r.range(start..end).ok_or(Error::SyntaxError)
}

/// Skip forward until the matching `]` is found.
///
/// Properly handles nested brackets, string literals, hex strings,
/// ASCII85 strings, and comments so that delimiters inside those
/// constructs are not mistaken for the closing bracket.
fn skip_to_matching_bracket(r: &mut Reader<'_>) -> Result<(), Error> {
    let mut depth = 1u32;

    while depth > 0 {
        let byte = r.read_byte().ok_or(Error::SyntaxError)?;

        match byte {
            b'[' => depth += 1,
            b']' => depth -= 1,
            // Literal string: skip to matching `)`, handling nesting/escapes.
            b'(' => skip_literal_string(r)?,
            b'<' => {
                match r.peek_byte() {
                    Some(b'~') => {
                        // ASCII85 string: skip to `~>`.
                        r.forward();
                        skip_ascii85_string(r)?;
                    }
                    Some(b'<') => {
                        // `<<` — just consume the second `<`.
                        r.forward();
                    }
                    _ => {
                        // Hex string: skip to `>`.
                        skip_hex_string(r)?;
                    }
                }
            }
            b'>' => {
                // `>>` — consume the second `>` if present.
                if r.peek_byte() == Some(b'>') {
                    r.forward();
                }
            }
            b'%' => {
                // Comment: skip to end of line.
                r.forward_while(|b| !crate::reader::is_eol(b));
            }
            _ => {}
        }
    }

    Ok(())
}

/// Skip a literal string body after the opening `(` has been consumed.
fn skip_literal_string(r: &mut Reader<'_>) -> Result<(), Error> {
    let mut depth = 1u32;
    while depth > 0 {
        let byte = r.read_byte().ok_or(Error::SyntaxError)?;
        match byte {
            b'\\' => {
                r.read_byte().ok_or(Error::SyntaxError)?;
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
    }
    Ok(())
}

/// Skip a hex string body after the opening `<` has been consumed.
fn skip_hex_string(r: &mut Reader<'_>) -> Result<(), Error> {
    loop {
        let byte = r.read_byte().ok_or(Error::SyntaxError)?;
        if byte == b'>' {
            return Ok(());
        }
    }
}

/// Skip an ASCII85 string body after the opening `<~` has been consumed.
fn skip_ascii85_string(r: &mut Reader<'_>) -> Result<(), Error> {
    loop {
        let byte = r.read_byte().ok_or(Error::SyntaxError)?;
        if byte == b'~' {
            if r.peek_byte() == Some(b'>') {
                r.forward();
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_array(input: &[u8]) -> Result<&[u8], Error> {
        let mut r = Reader::new(input);
        parse(&mut r)
    }

    #[test]
    fn empty() {
        assert_eq!(parse_array(b"[]").unwrap(), b"");
    }

    #[test]
    fn simple() {
        assert_eq!(parse_array(b"[1 2 3]").unwrap(), b"1 2 3");
    }

    #[test]
    fn nested() {
        assert_eq!(
            parse_array(b"[1 [2 3] 4]").unwrap(),
            b"1 [2 3] 4"
        );
    }

    #[test]
    fn with_string() {
        // The ']' inside the string should not close the array.
        assert_eq!(
            parse_array(b"[1 (str]) 2]").unwrap(),
            b"1 (str]) 2"
        );
    }

    #[test]
    fn with_hex_string() {
        assert_eq!(
            parse_array(b"[<48> /name]").unwrap(),
            b"<48> /name"
        );
    }

    #[test]
    fn with_ascii85_string() {
        assert_eq!(
            parse_array(b"[<~87cURDZ~> 1]").unwrap(),
            b"<~87cURDZ~> 1"
        );
    }

    #[test]
    fn with_comment() {
        assert_eq!(
            parse_array(b"[1 % comment with ]\n2]").unwrap(),
            b"1 % comment with ]\n2"
        );
    }

    #[test]
    fn unterminated() {
        assert_eq!(parse_array(b"[1 2"), Err(Error::SyntaxError));
    }

    #[test]
    fn not_an_array() {
        assert_eq!(parse_array(b"1 2]"), Err(Error::SyntaxError));
    }
}
