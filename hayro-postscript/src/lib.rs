/*!
A lightweight PostScript scanner.

This crate provides a scanner for tokenizing PostScript programs into typed objects.
It currently only implements a very small subset of the PostScript language,
with the main goal of being enough to parse CMAP files, but the scope _might_
be expanded upon in the future.

The supported types include integers and real numbers, name objects, strings and arrays.
Unsupported is anything else, including dictionaries, procedures, etc. An error
will be returned in case any of these is encountered.

## Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![no_std]
#![forbid(unsafe_code)]
#![allow(missing_docs)]

extern crate alloc;

mod array;
mod error;
mod name;
mod number;
mod object;
mod reader;
mod string;

pub use array::Array;
pub use error::{Error, Result};
pub use name::Name;
pub use number::Number;
pub use object::Object;
pub use string::String;

use reader::Reader;

/// A PostScript scanner that parses [`Object`]s from a byte stream.
pub struct Scanner<'a> {
    reader: Reader<'a>,
}

impl<'a> Scanner<'a> {
    /// Create a new scanner over the given bytes of a PostScript program.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(data),
        }
    }

    /// Returns `true` if there are no more objects to parse.
    pub fn at_end(&mut self) -> bool {
        object::at_end(&mut self.reader)
    }

    /// Parse the next object.
    pub fn parse_object(&mut self) -> Result<Object<'a>> {
        object::read(&mut self.reader)
    }

    /// Parse the next object as a [`Number`].
    pub fn parse_number(&mut self) -> Result<Number> {
        match self.parse_object()? {
            Object::Number(n) => Ok(n),
            _ => Err(Error::SyntaxError),
        }
    }

    /// Parse the next object as a [`Name`].
    pub fn parse_name(&mut self) -> Result<Name<'a>> {
        match self.parse_object()? {
            Object::Name(n) => Ok(n),
            _ => Err(Error::SyntaxError),
        }
    }

    /// Parse the next object as a [`String`].
    pub fn parse_string(&mut self) -> Result<String<'a>> {
        match self.parse_object()? {
            Object::String(s) => Ok(s),
            _ => Err(Error::SyntaxError),
        }
    }

    /// Parse the next object as an [`Array`].
    pub fn parse_array(&mut self) -> Result<Array<'a>> {
        match self.parse_object()? {
            Object::Array(a) => Ok(a),
            _ => Err(Error::SyntaxError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmap_snippet() {
        let input = br#"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CMapName /Test-H def
1 begincodespacerange
<00> <FF>
endcodespacerange
2 beginbfchar
<03> <0041>
<04> <0042>
endbfchar
endcmap"#;

        let mut s = Scanner::new(input);

        assert_eq!(s.parse_name().unwrap(), Name::new(b"CIDInit", true));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"ProcSet", true));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"findresource", false));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"begin", false));
        assert_eq!(s.parse_number().unwrap(), Number::Integer(12));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"dict", false));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"begin", false));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"begincmap", false));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"CMapName", true));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"Test-H", true));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"def", false));
        assert_eq!(s.parse_number().unwrap(), Number::Integer(1));
        assert_eq!(
            s.parse_name().unwrap(),
            Name::new(b"begincodespacerange", false)
        );
        assert_eq!(s.parse_string().unwrap(), String::from_hex(b"00"));
        assert_eq!(s.parse_string().unwrap(), String::from_hex(b"FF"));
        assert_eq!(
            s.parse_name().unwrap(),
            Name::new(b"endcodespacerange", false)
        );
        assert_eq!(s.parse_number().unwrap(), Number::Integer(2));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"beginbfchar", false));
        assert_eq!(s.parse_string().unwrap(), String::from_hex(b"03"));
        assert_eq!(s.parse_string().unwrap(), String::from_hex(b"0041"));
        assert_eq!(s.parse_string().unwrap(), String::from_hex(b"04"));
        assert_eq!(s.parse_string().unwrap(), String::from_hex(b"0042"));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"endbfchar", false));
        assert_eq!(s.parse_name().unwrap(), Name::new(b"endcmap", false));
        assert!(s.at_end());
    }

    #[test]
    fn array_round_trip() {
        let input = b"[123 /abc (xyz)]";
        let mut scanner = Scanner::new(input);
        let arr = scanner.parse_array().unwrap();
        assert!(scanner.at_end());

        let mut inner = arr.objects();
        assert_eq!(inner.parse_number().unwrap(), Number::Integer(123));
        assert_eq!(inner.parse_name().unwrap(), Name::new(b"abc", true));
        assert_eq!(inner.parse_string().unwrap(), String::from_literal(b"xyz"));
        assert!(inner.at_end());
    }

    #[test]
    fn comments_skipped() {
        let input = b"% comment\n42 % another\n/Name";
        let mut scanner = Scanner::new(input);

        assert_eq!(scanner.parse_number().unwrap(), Number::Integer(42));
        assert_eq!(scanner.parse_name().unwrap(), Name::new(b"Name", true));
        assert!(scanner.at_end());
    }

    #[test]
    fn wrong_type_is_error() {
        let mut scanner = Scanner::new(b"42 ");
        assert!(scanner.parse_name().is_err());
    }
}
