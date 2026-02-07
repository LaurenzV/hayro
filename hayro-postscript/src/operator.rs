//! Bare-word operator parsing.

use crate::object::Bytes;
use crate::reader::{Reader, is_regular};

/// Read a bare-word operator (regular characters until whitespace or delimiter).
///
/// The reader must be positioned at the first character of the operator.
pub(crate) fn read(r: &mut Reader<'_>) -> Option<Bytes> {
    let start = r.offset();
    r.forward_while(is_regular);

    if r.offset() == start {
        return None;
    }

    let bytes = r.range(start..r.offset())?;
    Some(Bytes::from_slice(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_op(input: &[u8]) -> Option<Bytes> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    #[test]
    fn simple() {
        assert_eq!(read_op(b"beginbfchar ").unwrap(), b"beginbfchar");
    }

    #[test]
    fn stops_at_delimiter() {
        let mut r = Reader::new(b"def/name");
        let op = read(&mut r).unwrap();
        assert_eq!(op, b"def");
        assert_eq!(r.peek_byte(), Some(b'/'));
    }

    #[test]
    fn at_eof() {
        assert_eq!(read_op(b"endcmap").unwrap(), b"endcmap");
    }

    #[test]
    fn empty() {
        assert!(read_op(b"").is_none());
    }

    #[test]
    fn starts_at_delimiter() {
        assert!(read_op(b"(foo)").is_none());
    }
}
