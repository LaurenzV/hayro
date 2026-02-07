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

/// A PostScript scanner that iterates over [`Object`]s in a byte stream.
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
}

impl<'a> Iterator for Scanner<'a> {
    type Item = Result<Object<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        object::read(&mut self.reader)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    fn collect_ok(input: &[u8]) -> Vec<Object<'_>> {
        Scanner::new(input).collect::<Result<Vec<_>>>().unwrap()
    }

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

        let objects = collect_ok(input);

        assert_eq!(objects[0], Object::Name(Name::new(b"CIDInit", true)));
        assert_eq!(objects[1], Object::Name(Name::new(b"ProcSet", true)));
        assert_eq!(objects[2], Object::Name(Name::new(b"findresource", false)));
        assert_eq!(objects[3], Object::Name(Name::new(b"begin", false)));
        assert_eq!(objects[4], Object::Number(Number::Integer(12)));
        assert_eq!(objects[5], Object::Name(Name::new(b"dict", false)));
        assert_eq!(objects[6], Object::Name(Name::new(b"begin", false)));
        assert_eq!(objects[7], Object::Name(Name::new(b"begincmap", false)));
        assert_eq!(objects[8], Object::Name(Name::new(b"CMapName", true)));
        assert_eq!(objects[9], Object::Name(Name::new(b"Test-H", true)));
        assert_eq!(objects[10], Object::Name(Name::new(b"def", false)));
        assert_eq!(objects[11], Object::Number(Number::Integer(1)));
        assert_eq!(
            objects[12],
            Object::Name(Name::new(b"begincodespacerange", false))
        );
        assert_eq!(objects[13], Object::String(String::from_hex(b"00")));
        assert_eq!(objects[14], Object::String(String::from_hex(b"FF")));
        assert_eq!(
            objects[15],
            Object::Name(Name::new(b"endcodespacerange", false))
        );
        assert_eq!(objects[16], Object::Number(Number::Integer(2)));
        assert_eq!(objects[17], Object::Name(Name::new(b"beginbfchar", false)));
        assert_eq!(objects[18], Object::String(String::from_hex(b"03")));
        assert_eq!(objects[19], Object::String(String::from_hex(b"0041")));
        assert_eq!(objects[20], Object::String(String::from_hex(b"04")));
        assert_eq!(objects[21], Object::String(String::from_hex(b"0042")));
        assert_eq!(objects[22], Object::Name(Name::new(b"endbfchar", false)));
        assert_eq!(objects[23], Object::Name(Name::new(b"endcmap", false)));
        assert_eq!(objects.len(), 24);
    }

    #[test]
    fn array_round_trip() {
        let input = b"[123 /abc (xyz)]";
        let objects = collect_ok(input);
        assert_eq!(objects.len(), 1);

        if let Object::Array(arr) = &objects[0] {
            let mut inner = arr.objects();
            assert_eq!(
                inner.next().unwrap().unwrap(),
                Object::Number(Number::Integer(123))
            );
            assert_eq!(
                inner.next().unwrap().unwrap(),
                Object::Name(Name::new(b"abc", true))
            );
            assert_eq!(
                inner.next().unwrap().unwrap(),
                Object::String(String::from_literal(b"xyz"))
            );
            assert!(inner.next().is_none());
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn comments_skipped() {
        let input = b"% comment\n42 % another\n/Name";
        let objects = collect_ok(input);

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0], Object::Number(Number::Integer(42)));
        assert_eq!(objects[1], Object::Name(Name::new(b"Name", true)));
    }
}
