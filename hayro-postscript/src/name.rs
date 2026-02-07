//! Name `/...` parsing.

use crate::string::decode_hex_digit;
use crate::object::Bytes;
use crate::reader::{Reader, is_regular};

/// Read a name object `/...`, returning the decoded bytes.
///
/// The reader must be positioned at the leading `/`.
pub(crate) fn read(r: &mut Reader<'_>) -> Option<Bytes> {
    r.forward_tag(b"/")?;

    let start = r.offset();

    // Scan for the extent of the name.
    while r.eat(is_regular).is_some() {}

    let raw = r.range(start..r.offset())?;

    // Fast path: no `#` escapes.
    if !raw.contains(&b'#') {
        return Some(Bytes::from_slice(raw));
    }

    // Slow path: decode `#XX` hex escapes.
    let mut result = Bytes::with_capacity(raw.len());
    let mut inner = Reader::new(raw);

    while let Some(b) = inner.read_byte() {
        if b == b'#' {
            let hex = inner.read_bytes(2)?;
            result.push(decode_hex_digit(hex[0])? << 4 | decode_hex_digit(hex[1])?);
        } else {
            result.push(b);
        }
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_name(input: &[u8]) -> Option<Bytes> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    #[test]
    fn simple() {
        assert_eq!(&*read_name(b"/Name1").unwrap(), b"Name1");
    }

    #[test]
    fn empty_name() {
        assert_eq!(&*read_name(b"/").unwrap(), b"");
    }

    #[test]
    fn with_hex_escape() {
        assert_eq!(&*read_name(b"/lime#20Green").unwrap(), b"lime Green");
    }

    #[test]
    fn multiple_hex_escapes() {
        assert_eq!(
            &*read_name(b"/paired#28#29parentheses").unwrap(),
            b"paired()parentheses"
        );
    }

    #[test]
    fn special_chars() {
        assert_eq!(
            &*read_name(b"/A;Name_With-Various***Characters?").unwrap(),
            b"A;Name_With-Various***Characters?"
        );
    }

    #[test]
    fn stops_at_delimiter() {
        let mut r = Reader::new(b"/Name(rest");
        let name = read(&mut r).unwrap();
        assert_eq!(&*name, b"Name");
        assert_eq!(r.peek_byte(), Some(b'('));
    }

    #[test]
    fn stops_at_whitespace() {
        let mut r = Reader::new(b"/Name rest");
        let name = read(&mut r).unwrap();
        assert_eq!(&*name, b"Name");
        assert_eq!(r.peek_byte(), Some(b' '));
    }

    #[test]
    fn not_a_name() {
        assert!(read_name(b"Name").is_none());
    }
}
