/*!
A lightweight PostScript scanner.

This crate provides a scanner for tokenizing PostScript programs into typed tokens.
It currently only implements a very small subset of the PostScript language,
with the main goal of being enough to parse CMAP files, but the scope _might_
be expanded upon in the future.

The supported tokens are integers and real numbers, names, strings and
procedures. The `[`, `]`, `<<` and `>>` tokens are returned as executable names, so
that the caller can construct arrays and dictionaries if needed.

## Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![no_std]
#![forbid(unsafe_code)]
#![allow(missing_docs)]

extern crate alloc;

mod error;
mod name;
mod number;
mod procedure;
mod reader;
mod string;
mod token;

pub use error::{Error, Result};
pub use name::Name;
pub use number::Number;
pub use procedure::Procedure;
pub use string::String;
pub use token::Token;

use reader::Reader;

/// A PostScript scanner that parses [`Token`]s from a byte stream.
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

    /// Return the current byte offset into the data the scanner was created with.
    pub fn offset(&self) -> usize {
        self.reader.offset()
    }

    /// Move the scanner to a byte offset into the data it was created with.
    pub fn set_offset(&mut self, offset: usize) {
        self.reader.jump(offset);
    }

    /// Returns `true` if there are no more tokens to parse.
    pub fn at_end(&mut self) -> bool {
        token::at_end(&mut self.reader)
    }

    /// Parse the next token.
    pub fn parse_token(&mut self) -> Result<Token<'a>> {
        token::read(&mut self.reader)
    }

    /// Parse the next token as a [`Number`].
    pub fn parse_number(&mut self) -> Result<Number> {
        match self.parse_token()? {
            Token::Number(n) => Ok(n),
            _ => Err(Error::SyntaxError),
        }
    }

    /// Parse the next token as a [`Name`].
    pub fn parse_name(&mut self) -> Result<Name<'a>> {
        match self.parse_token()? {
            Token::Name(n) => Ok(n),
            _ => Err(Error::SyntaxError),
        }
    }

    /// Parse the next token as a [`String`].
    pub fn parse_string(&mut self) -> Result<String<'a>> {
        match self.parse_token()? {
            Token::String(s) => Ok(s),
            _ => Err(Error::SyntaxError),
        }
    }

    /// Parse the next token as a [`Procedure`].
    pub fn parse_procedure(&mut self) -> Result<Procedure<'a>> {
        match self.parse_token()? {
            Token::Procedure(p) => Ok(p),
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
    fn array_tokens() {
        let mut scanner = Scanner::new(b"[123 /abc (xyz)]");

        assert_eq!(scanner.parse_name().unwrap(), Name::new(b"[", false));
        assert_eq!(scanner.parse_number().unwrap(), Number::Integer(123));
        assert_eq!(scanner.parse_name().unwrap(), Name::new(b"abc", true));
        assert_eq!(
            scanner.parse_string().unwrap(),
            String::from_literal(b"xyz")
        );
        assert_eq!(scanner.parse_name().unwrap(), Name::new(b"]", false));
        assert!(scanner.at_end());
    }

    #[test]
    fn procedure_round_trip() {
        let input = b"{dup 0 lt {neg} if}";
        let mut scanner = Scanner::new(input);
        let proc = scanner.parse_procedure().unwrap();
        assert!(scanner.at_end());

        let mut inner = proc.tokens();
        assert_eq!(inner.parse_name().unwrap(), Name::new(b"dup", false));
        assert_eq!(inner.parse_number().unwrap(), Number::Integer(0));
        assert_eq!(inner.parse_name().unwrap(), Name::new(b"lt", false));
        let nested = inner.parse_procedure().unwrap();
        assert_eq!(inner.parse_name().unwrap(), Name::new(b"if", false));
        assert!(inner.at_end());

        let mut nested = nested.tokens();
        assert_eq!(nested.parse_name().unwrap(), Name::new(b"neg", false));
        assert!(nested.at_end());
    }

    #[test]
    fn immediate_name_in_procedure() {
        let mut scanner = Scanner::new(b"{//mydict begin}");
        let mut inner = scanner.parse_procedure().unwrap().tokens();

        let name = inner.parse_name().unwrap();
        assert!(name.is_immediate());
        assert!(!name.is_literal());
        assert_eq!(name.as_bytes(), b"mydict");
        assert_eq!(inner.parse_name().unwrap(), Name::new(b"begin", false));
        assert!(inner.at_end());
    }

    #[test]
    fn dict_tokens() {
        let mut scanner = Scanner::new(b"<</Registry (Adobe)>>");

        assert_eq!(scanner.parse_name().unwrap(), Name::new(b"<<", false));
        assert_eq!(scanner.parse_name().unwrap(), Name::new(b"Registry", true));
        assert_eq!(
            scanner.parse_string().unwrap(),
            String::from_literal(b"Adobe")
        );
        assert_eq!(scanner.parse_name().unwrap(), Name::new(b">>", false));
        assert!(scanner.at_end());
    }

    #[test]
    fn offset_after_token() {
        let input = b"/Private 4 dict eexec \x01\x02";
        let mut scanner = Scanner::new(input);
        assert_eq!(scanner.offset(), 0);

        assert_eq!(scanner.parse_name(), Ok(Name::new(b"Private", true)));
        assert_eq!(scanner.offset(), 8);
        assert_eq!(scanner.parse_number(), Ok(Number::Integer(4)));
        assert_eq!(scanner.parse_name(), Ok(Name::new(b"dict", false)));
        assert_eq!(scanner.parse_name(), Ok(Name::new(b"eexec", false)));
        assert_eq!(scanner.offset(), input.len() - 3);
    }

    #[test]
    fn set_offset_skips_binary_data() {
        let input = b"dup 0 4 RD {(<% NP";
        let mut scanner = Scanner::new(input);

        assert_eq!(scanner.parse_name(), Ok(Name::new(b"dup", false)));
        assert_eq!(scanner.parse_number(), Ok(Number::Integer(0)));
        assert_eq!(scanner.parse_number(), Ok(Number::Integer(4)));
        assert_eq!(scanner.parse_name(), Ok(Name::new(b"RD", false)));

        let start = scanner.offset() + 1;
        assert_eq!(&input[start..start + 4], b"{(<%");

        scanner.set_offset(start + 4);
        assert_eq!(scanner.parse_name(), Ok(Name::new(b"NP", false)));
        assert!(scanner.at_end());
    }

    #[test]
    fn set_offset_past_end() {
        let mut scanner = Scanner::new(b"1 2");
        scanner.set_offset(10);
        assert!(scanner.at_end());
        assert!(scanner.parse_token().is_err());
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
