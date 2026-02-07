//! Object enum and top-level read dispatch.

use crate::array::{self, Array};
use crate::error::{Error, Result};
use crate::name::{self, Name};
use crate::number::{self, Number};
use crate::reader::Reader;
use crate::string::{self, String};

/// A PostScript object.
#[derive(Debug, Clone, PartialEq)]
pub enum Object<'a> {
    /// An integer value.
    Integer(i32),
    /// A real (floating-point) value.
    Real(f64),
    /// A name object — either literal (`/foo`) or executable (`foo`).
    Name(Name<'a>),
    /// A string object (literal, hex, or ASCII85).
    String(String<'a>),
    /// An array object `[...]`.
    Array(Array<'a>),
}

/// Try to read the next PostScript object from the reader.
///
/// Whitespace and comments are skipped before dispatching.
/// Returns `None` at EOF, `Some(Ok(..))` on success, `Some(Err(..))`
/// on error.
pub(crate) fn read<'a>(r: &mut Reader<'a>) -> Option<Result<Object<'a>>> {
    skip_whitespace_and_comments(r);

    let b = r.peek_byte()?;

    Some(match b {
        b'(' => string::parse_literal(r)
            .map(|s| Object::String(String::from_literal(s)))
            .ok_or(Error::SyntaxError),
        b'<' => {
            // Check for `<~` (ASCII85) before `<<` (dict).
            if r.peek_bytes(2) == Some(b"<~") {
                string::parse_ascii85(r)
                    .map(|s| Object::String(String::from_ascii85(s)))
                    .ok_or(Error::SyntaxError)
            } else if r.peek_bytes(2) == Some(b"<<") {
                r.forward();
                r.forward();
                Err(Error::UnsupportedType)
            } else {
                string::parse_hex(r)
                    .map(|s| Object::String(String::from_hex(s)))
                    .ok_or(Error::SyntaxError)
            }
        }
        b'>' => {
            if r.peek_bytes(2) == Some(b">>") {
                r.forward();
                r.forward();
                Err(Error::UnsupportedType)
            } else {
                r.forward();
                Err(Error::SyntaxError)
            }
        }
        b'/' => name::parse_literal(r)
            .map(|s| Object::Name(Name::new(s, true)))
            .ok_or(Error::SyntaxError),
        b'[' => array::parse(r).map(|d| Object::Array(Array::new(d))),
        b']' => {
            r.forward();
            Err(Error::SyntaxError)
        }
        b'{' | b'}' => {
            r.forward();
            Err(Error::UnsupportedType)
        }
        b'.' => {
            if let Some(n) = number::read(r) {
                Ok(match n {
                    Number::Integer(v) => Object::Integer(v),
                    Number::Real(v) => Object::Real(v),
                })
            } else {
                name::parse_executable(r)
                    .map(|s| Object::Name(Name::new(s, false)))
                    .ok_or(Error::SyntaxError)
            }
        }
        b'+' | b'-' | b'0'..=b'9' => {
            if let Some(n) = number::read(r) {
                Ok(match n {
                    Number::Integer(v) => Object::Integer(v),
                    Number::Real(v) => Object::Real(v),
                })
            } else {
                name::parse_executable(r)
                    .map(|s| Object::Name(Name::new(s, false)))
                    .ok_or(Error::SyntaxError)
            }
        }
        _ => name::parse_executable(r)
            .map(|s| Object::Name(Name::new(s, false)))
            .ok_or(Error::SyntaxError),
    })
}

pub(crate) fn skip_whitespace_and_comments(r: &mut Reader<'_>) {
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

    fn read_one(input: &[u8]) -> Option<Result<Object<'_>>> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    fn read_ok(input: &[u8]) -> Object<'_> {
        read_one(input).unwrap().unwrap()
    }

    fn read_err(input: &[u8]) -> Error {
        read_one(input).unwrap().unwrap_err()
    }

    #[test]
    fn integer() {
        assert_eq!(read_ok(b"42 "), Object::Integer(42));
    }

    #[test]
    fn negative_integer() {
        assert_eq!(read_ok(b"-7 "), Object::Integer(-7));
    }

    #[test]
    fn literal_name() {
        assert_eq!(
            read_ok(b"/CMapName "),
            Object::Name(Name::new(b"CMapName", true))
        );
    }

    #[test]
    fn executable_name() {
        let obj = read_ok(b"beginbfchar ");
        assert_eq!(obj, Object::Name(Name::new(b"beginbfchar", false)));
    }

    #[test]
    fn literal_string() {
        assert_eq!(
            read_ok(b"(Hello)"),
            Object::String(String::from_literal(b"Hello"))
        );
    }

    #[test]
    fn hex_string() {
        assert_eq!(
            read_ok(b"<48656C6C6F>"),
            Object::String(String::from_hex(b"48656C6C6F"))
        );
    }

    #[test]
    fn ascii85_string() {
        assert_eq!(
            read_ok(b"<~87cURDZ~>"),
            Object::String(String::from_ascii85(b"87cURDZ"))
        );
    }

    #[test]
    fn array_simple() {
        let obj = read_ok(b"[1 2 3]");
        assert_eq!(obj, Object::Array(Array::new(b"1 2 3")));
    }

    #[test]
    fn dict_open_error() {
        assert_eq!(read_err(b"<< "), Error::UnsupportedType);
    }

    #[test]
    fn dict_close_error() {
        assert_eq!(read_err(b">> "), Error::UnsupportedType);
    }

    #[test]
    fn procedure_open_error() {
        assert_eq!(read_err(b"{ "), Error::UnsupportedType);
    }

    #[test]
    fn procedure_close_error() {
        assert_eq!(read_err(b"} "), Error::UnsupportedType);
    }

    #[test]
    fn stray_close_bracket() {
        assert_eq!(read_err(b"]"), Error::SyntaxError);
    }

    #[test]
    fn stray_gt() {
        assert_eq!(read_err(b">x"), Error::SyntaxError);
    }

    #[test]
    fn skips_whitespace() {
        assert_eq!(read_ok(b"  \t\n 42 "), Object::Integer(42));
    }

    #[test]
    fn skips_comments() {
        assert_eq!(read_ok(b"% this is a comment\n42 "), Object::Integer(42));
    }

    #[test]
    fn eof() {
        assert_eq!(read_one(b""), None);
        assert_eq!(read_one(b"   "), None);
        assert_eq!(read_one(b"% comment only\n"), None);
    }
}
