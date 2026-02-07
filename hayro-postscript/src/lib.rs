/*!
A lightweight PostScript scanner.

This crate provides a scanner for tokenizing PostScript programs into typed objects.
It currently implements a small subset of the PostScript language, focused on what
is needed to support CMap parsing in PDF documents.

## Supported object types
- **Integers** — signed integers, radix numbers (e.g. `8#1777`, `16#FFFE`)
- **Reals** — floating-point numbers with optional exponent (e.g. `34.5`, `1.0E-5`)
- **Names** — literal (`/Name`) and executable (`Name`), with `#XX` hex-escape decoding
- **Strings** — literal `(...)`, hex `<...>`, and ASCII85 `<~...~>`, with lazy decoding
- **Arrays** — `[...]` with lazy inner object iteration

## Limitations
This crate only implements a small subset of the PostScript language. In particular,
the following features are **not** supported:
- Dictionaries (`<<` / `>>`)
- Procedures (`{` / `}`)
- The PostScript execution model (operand/dictionary stacks, operators, etc.)

Encountering unsupported syntax returns an error rather than silently skipping it.

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
pub use object::Object;
pub use string::String;

use reader::Reader;

/// A PostScript scanner that iterates over [`Object`]s in a byte stream.
pub struct Scanner<'a> {
    reader: Reader<'a>,
}

impl<'a> Scanner<'a> {
    /// Create a new scanner over the given bytes.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(data),
        }
    }

    /// Read the next object from the stream.
    ///
    /// Returns `Ok(None)` at EOF, `Ok(Some(..))` on success, `Err(..)`
    /// on error.
    pub fn next(&mut self) -> Result<Option<Object<'a>>> {
        object::read(&mut self.reader)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    fn collect_ok(input: &[u8]) -> Vec<Object<'_>> {
        let mut scanner = Scanner::new(input);
        let mut objects = Vec::new();
        while let Some(obj) = scanner.next().unwrap() {
            objects.push(obj);
        }
        objects
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
        assert_eq!(objects[4], Object::Integer(12));
        assert_eq!(objects[5], Object::Name(Name::new(b"dict", false)));
        assert_eq!(objects[6], Object::Name(Name::new(b"begin", false)));
        assert_eq!(objects[7], Object::Name(Name::new(b"begincmap", false)));
        assert_eq!(objects[8], Object::Name(Name::new(b"CMapName", true)));
        assert_eq!(objects[9], Object::Name(Name::new(b"Test-H", true)));
        assert_eq!(objects[10], Object::Name(Name::new(b"def", false)));
        assert_eq!(objects[11], Object::Integer(1));
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
        assert_eq!(objects[16], Object::Integer(2));
        assert_eq!(
            objects[17],
            Object::Name(Name::new(b"beginbfchar", false))
        );
        assert_eq!(objects[18], Object::String(String::from_hex(b"03")));
        assert_eq!(objects[19], Object::String(String::from_hex(b"0041")));
        assert_eq!(objects[20], Object::String(String::from_hex(b"04")));
        assert_eq!(objects[21], Object::String(String::from_hex(b"0042")));
        assert_eq!(
            objects[22],
            Object::Name(Name::new(b"endbfchar", false))
        );
        assert_eq!(
            objects[23],
            Object::Name(Name::new(b"endcmap", false))
        );
        assert_eq!(objects.len(), 24);
    }

    #[test]
    fn dict_delimiters_error() {
        let input = b"<< /Registry (Adobe) >>";
        let mut scanner = Scanner::new(input);

        assert_eq!(scanner.next(), Err(Error::UnsupportedType)); // <<
        assert_eq!(scanner.next().unwrap(), Some(Object::Name(Name::new(b"Registry", true))));
        assert_eq!(
            scanner.next().unwrap(),
            Some(Object::String(String::from_literal(b"Adobe")))
        );
        assert_eq!(scanner.next(), Err(Error::UnsupportedType)); // >>
        assert_eq!(scanner.next().unwrap(), None);
    }

    #[test]
    fn array_round_trip() {
        let input = b"[123 /abc (xyz)]";
        let objects = collect_ok(input);
        assert_eq!(objects.len(), 1);

        if let Object::Array(arr) = &objects[0] {
            let inner = collect_ok(arr.data());
            assert_eq!(inner.len(), 3);
            assert_eq!(inner[0], Object::Integer(123));
            assert_eq!(inner[1], Object::Name(Name::new(b"abc", true)));
            assert_eq!(inner[2], Object::String(String::from_literal(b"xyz")));
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn comments_skipped() {
        let input = b"% comment\n42 % another\n/Name";
        let objects = collect_ok(input);

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0], Object::Integer(42));
        assert_eq!(objects[1], Object::Name(Name::new(b"Name", true)));
    }

    #[test]
    fn procedure_error() {
        let mut scanner = Scanner::new(b"{ }");
        assert_eq!(scanner.next(), Err(Error::UnsupportedType));
        assert_eq!(scanner.next(), Err(Error::UnsupportedType));
        assert_eq!(scanner.next().unwrap(), None);
    }
}
