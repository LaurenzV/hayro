use crate::error::{Error, Result};
use crate::reader::Reader;
use crate::string;
use crate::token;

/// A PostScript procedure (executable array) object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Procedure<'a> {
    data: &'a [u8],
}

impl<'a> Procedure<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Return a [`Scanner`](crate::Scanner) that iterates over the tokens inside
    /// this procedure.
    pub fn tokens(&self) -> crate::Scanner<'a> {
        crate::Scanner::new(self.data)
    }
}

pub(crate) fn parse<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    r.forward_tag(b"{").ok_or(Error::SyntaxError)?;

    let start = r.offset();
    skip(r)?;
    let end = r.offset() - 1;

    r.range(start..end).ok_or(Error::SyntaxError)
}

fn skip(r: &mut Reader<'_>) -> Result<()> {
    let mut depth = 1_u32;

    while depth > 0 {
        match r.peek_byte().ok_or(Error::SyntaxError)? {
            b'{' => {
                r.forward();
                depth += 1;
            }
            b'}' => {
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
            b'%' => token::skip_whitespace_and_comments(r),
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

    fn parse_procedure(input: &[u8]) -> Result<&[u8]> {
        let mut r = Reader::new(input);
        parse(&mut r)
    }

    #[test]
    fn empty() {
        assert_eq!(parse_procedure(b"{}").unwrap(), b"");
    }

    #[test]
    fn simple() {
        assert_eq!(parse_procedure(b"{add 2 div}").unwrap(), b"add 2 div");
    }

    #[test]
    fn nested() {
        assert_eq!(
            parse_procedure(b"{dup 0 lt {neg} if}").unwrap(),
            b"dup 0 lt {neg} if"
        );
    }

    #[test]
    fn unbalanced_brackets() {
        assert_eq!(parse_procedure(b"{[ 1 2}").unwrap(), b"[ 1 2");
        assert_eq!(parse_procedure(b"{] 1}").unwrap(), b"] 1");
    }

    #[test]
    fn with_strings() {
        assert_eq!(
            parse_procedure(b"{(}) <7D> <~Ac~>}").unwrap(),
            b"(}) <7D> <~Ac~>"
        );
    }

    #[test]
    fn with_comment() {
        assert_eq!(
            parse_procedure(b"{1 % comment with }\n2}").unwrap(),
            b"1 % comment with }\n2"
        );
    }

    #[test]
    fn unterminated() {
        assert_eq!(parse_procedure(b"{1 {2}"), Err(Error::SyntaxError));
    }

    #[test]
    fn deeply_nested_unterminated() {
        let input = b"{".repeat(1_000_000);
        assert_eq!(parse_procedure(&input), Err(Error::SyntaxError));
    }
}
