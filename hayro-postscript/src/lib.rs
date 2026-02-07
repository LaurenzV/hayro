/*!
A lightweight PostScript parser and interpreter.

This crate provides parsing and interpretation of PostScript programs,
focused on the subset commonly used in PDF documents (Type 1 fonts,
CFF/Type 2 charstrings, etc.).

The crate is `no_std` compatible but requires an allocator to be available.

# Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![allow(missing_docs)]

extern crate alloc;

mod name;
mod number;
mod object;
mod operator;
mod reader;
mod string;

pub use object::{Bytes, Object};
pub use string::StringKind;

use reader::Reader;

/// A PostScript lexer that iterates over [`Object`]s in a byte stream.
pub struct Lexer<'a> {
    reader: Reader<'a>,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer over the given bytes.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(data),
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Object<'a>;

    fn next(&mut self) -> Option<Object<'a>> {
        object::read(&mut self.reader)
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

        let objects: Vec<Object<'_>> = Lexer::new(input).collect();

        // Spot-check a few objects.
        assert_eq!(objects[0], Object::Name(bytes!(b"CIDInit")));
        assert_eq!(objects[1], Object::Name(bytes!(b"ProcSet")));
        assert_eq!(objects[2], Object::Operator(bytes!(b"findresource")));
        assert_eq!(objects[3], Object::Operator(bytes!(b"begin")));
        assert_eq!(objects[4], Object::Integer(12));
        assert_eq!(objects[5], Object::Operator(bytes!(b"dict")));
        assert_eq!(objects[6], Object::Operator(bytes!(b"begin")));
        assert_eq!(objects[7], Object::Operator(bytes!(b"begincmap")));
        assert_eq!(objects[8], Object::Name(bytes!(b"CMapName")));
        assert_eq!(objects[9], Object::Name(bytes!(b"Test-H")));
        assert_eq!(objects[10], Object::Operator(bytes!(b"def")));
        assert_eq!(objects[11], Object::Integer(1));
        assert_eq!(
            objects[12],
            Object::Operator(bytes!(b"begincodespacerange"))
        );
        assert_eq!(objects[13], Object::String(StringKind::Hex(b"00")));
        assert_eq!(objects[14], Object::String(StringKind::Hex(b"FF")));
        assert_eq!(
            objects[15],
            Object::Operator(bytes!(b"endcodespacerange"))
        );
        assert_eq!(objects[16], Object::Integer(2));
        assert_eq!(objects[17], Object::Operator(bytes!(b"beginbfchar")));
        assert_eq!(objects[18], Object::String(StringKind::Hex(b"03")));
        assert_eq!(objects[19], Object::String(StringKind::Hex(b"0041")));
        assert_eq!(objects[20], Object::String(StringKind::Hex(b"04")));
        assert_eq!(objects[21], Object::String(StringKind::Hex(b"0042")));
        assert_eq!(objects[22], Object::Operator(bytes!(b"endbfchar")));
        assert_eq!(objects[23], Object::Operator(bytes!(b"endcmap")));
        assert_eq!(objects.len(), 24);
    }

    #[test]
    fn dict_delimiters() {
        let input = b"<< /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def";
        let objects: Vec<Object<'_>> = Lexer::new(input).collect();

        assert_eq!(objects[0], Object::Operator(bytes!(b"<<")));
        assert_eq!(objects[1], Object::Name(bytes!(b"Registry")));
        assert_eq!(objects[2], Object::String(StringKind::Literal(b"Adobe")));
        assert_eq!(objects[3], Object::Name(bytes!(b"Ordering")));
        assert_eq!(
            objects[4],
            Object::String(StringKind::Literal(b"Identity"))
        );
        assert_eq!(objects[5], Object::Name(bytes!(b"Supplement")));
        assert_eq!(objects[6], Object::Integer(0));
        assert_eq!(objects[7], Object::Operator(bytes!(b">>")));
        assert_eq!(objects[8], Object::Operator(bytes!(b"def")));
    }

    #[test]
    fn comments_skipped() {
        let input = b"% comment\n42 % another\n/Name";
        let objects: Vec<Object<'_>> = Lexer::new(input).collect();

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0], Object::Integer(42));
        assert_eq!(objects[1], Object::Name(bytes!(b"Name")));
    }
}
