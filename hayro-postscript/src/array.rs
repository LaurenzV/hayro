use crate::error::{Error, Result};
use crate::object;
use crate::reader::Reader;
use crate::string;

/// A PostScript array object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Array<'a> {
    data: &'a [u8],
}

impl<'a> Array<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Return a [`Scanner`](crate::Scanner) that iterates over the objects inside
    /// this array.
    pub fn objects(&self) -> crate::Scanner<'a> {
        crate::Scanner::new(self.data)
    }
}

pub(crate) fn parse<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    r.forward_tag(b"[").ok_or(Error::SyntaxError)?;

    let start = r.offset();
    skip_array(r)?;
    let end = r.offset() - 1;

    r.range(start..end).ok_or(Error::SyntaxError)
}

fn skip_array(r: &mut Reader<'_>) -> Result<()> {
    let mut depth = 1_u32;

    while depth > 0 {
        match r.peek_byte().ok_or(Error::SyntaxError)? {
            b'[' => {
                r.forward();
                depth += 1;
            }
            b']' => {
                r.forward();
                depth -= 1;
            }
            b'(' => {
                let _ = string::parse_literal(r).ok_or(Error::SyntaxError)?;
            }
            b'<' => {
                if r.peek_bytes(2) == Some(b"<~") {
                    let _ = string::parse_ascii85(r).ok_or(Error::SyntaxError)?;
                } else if r.peek_bytes(2) == Some(b"<<") {
                    r.forward();
                    r.forward();
                } else {
                    let _ = string::parse_hex(r).ok_or(Error::SyntaxError)?;
                }
            }
            b'%' => object::skip_whitespace_and_comments(r),
            _ => {
                r.forward();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_array(input: &[u8]) -> Result<&[u8]> {
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
        assert_eq!(parse_array(b"[1 [2 3] 4]").unwrap(), b"1 [2 3] 4");
    }

    #[test]
    fn with_string() {
        // The ']' inside the string should not close the array.
        assert_eq!(parse_array(b"[1 (str]) 2]").unwrap(), b"1 (str]) 2");
    }

    #[test]
    fn with_hex_string() {
        assert_eq!(parse_array(b"[<48> /name]").unwrap(), b"<48> /name");
    }

    #[test]
    fn with_ascii85_string() {
        assert_eq!(parse_array(b"[<~87cURDZ~> 1]").unwrap(), b"<~87cURDZ~> 1");
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
