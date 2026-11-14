use crate::reader::{Reader, is_regular};

/// A PostScript name object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Name<'a> {
    data: &'a [u8],
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Executable,
    Literal,
    Immediate,
}

impl<'a> Name<'a> {
    pub(crate) fn new(data: &'a [u8], literal: bool) -> Self {
        let kind = if literal {
            Kind::Literal
        } else {
            Kind::Executable
        };

        Self { data, kind }
    }

    pub(crate) fn new_immediate(data: &'a [u8]) -> Self {
        Self {
            data,
            kind: Kind::Immediate,
        }
    }

    /// Returns `true` if this is a literal name.
    pub fn is_literal(&self) -> bool {
        self.kind == Kind::Literal
    }

    /// Returns `true` if this is an immediately evaluated name.
    pub fn is_immediate(&self) -> bool {
        self.kind == Kind::Immediate
    }

    /// Returns the name as a string if it is valid UTF-8.
    pub fn as_str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.data).ok()
    }

    /// Returns the raw bytes of the name.
    pub fn as_bytes(&self) -> &'a [u8] {
        self.data
    }
}

pub(crate) fn parse_literal<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    r.forward_tag(b"/")?;
    let start = r.offset();
    while r.eat(is_regular).is_some() {}
    r.range(start..r.offset())
}

pub(crate) fn parse_immediate<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    r.forward_tag(b"/")?;
    parse_literal(r)
}

pub(crate) fn parse_executable<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    let start = r.offset();
    r.forward_while(is_regular);
    if r.offset() == start {
        return None;
    }
    r.range(start..r.offset())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_literal(input: &[u8]) -> Option<Name<'_>> {
        let mut r = Reader::new(input);
        parse_literal(&mut r).map(|d| Name::new(d, true))
    }

    fn read_executable(input: &[u8]) -> Option<Name<'_>> {
        let mut r = Reader::new(input);
        parse_executable(&mut r).map(|d| Name::new(d, false))
    }

    #[test]
    fn literal_simple() {
        let n = read_literal(b"/Name1").unwrap();
        assert_eq!(n.as_str().unwrap(), "Name1");
        assert!(n.is_literal());
    }

    #[test]
    fn literal_empty_name() {
        let n = read_literal(b"/").unwrap();
        assert_eq!(n.as_str().unwrap(), "");
        assert!(n.is_literal());
    }

    #[test]
    fn literal_with_number_sign() {
        let n = read_literal(b"/lime#20Green").unwrap();
        assert_eq!(n.as_bytes(), b"lime#20Green");
    }

    #[test]
    fn literal_special_chars() {
        let n = read_literal(b"/A;Name_With-Various***Characters?").unwrap();
        assert_eq!(n.as_str().unwrap(), "A;Name_With-Various***Characters?");
    }

    #[test]
    fn literal_stops_at_delimiter() {
        let mut r = Reader::new(b"/Name(rest");
        let data = parse_literal(&mut r).unwrap();
        assert_eq!(data, b"Name");
        assert_eq!(r.peek_byte(), Some(b'('));
    }

    #[test]
    fn literal_stops_at_whitespace() {
        let mut r = Reader::new(b"/Name rest");
        let data = parse_literal(&mut r).unwrap();
        assert_eq!(data, b"Name");
        assert_eq!(r.peek_byte(), Some(b' '));
    }

    #[test]
    fn literal_not_a_name() {
        assert!(read_literal(b"Name").is_none());
    }

    #[test]
    fn immediate_simple() {
        let mut r = Reader::new(b"//Helvetica ");
        assert_eq!(parse_immediate(&mut r), Some(&b"Helvetica"[..]));
        assert_eq!(r.peek_byte(), Some(b' '));
    }

    #[test]
    fn immediate_empty() {
        let mut r = Reader::new(b"//(x)");
        assert_eq!(parse_immediate(&mut r), Some(&b""[..]));
        assert_eq!(r.peek_byte(), Some(b'('));
    }

    #[test]
    fn executable_simple() {
        let n = read_executable(b"beginbfchar ").unwrap();
        assert_eq!(n.as_str().unwrap(), "beginbfchar");
        assert!(!n.is_literal());
    }

    #[test]
    fn executable_stops_at_delimiter() {
        let mut r = Reader::new(b"def/name");
        let data = parse_executable(&mut r).unwrap();
        assert_eq!(data, b"def");
        assert_eq!(r.peek_byte(), Some(b'/'));
    }

    #[test]
    fn executable_at_eof() {
        let n = read_executable(b"endcmap").unwrap();
        assert_eq!(n.as_str().unwrap(), "endcmap");
    }

    #[test]
    fn executable_empty() {
        assert!(read_executable(b"").is_none());
    }

    #[test]
    fn executable_starts_at_delimiter() {
        assert!(read_executable(b"(foo)").is_none());
    }
}
