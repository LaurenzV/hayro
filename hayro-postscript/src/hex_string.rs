//! Hex string `<...>` parsing and ASCII hex decoding.

use crate::object::Bytes;
use crate::reader::{Reader, is_whitespace};

/// Read a hex string `<...>`, returning the decoded bytes.
///
/// The reader must be positioned at the opening `<`.
pub(crate) fn read(r: &mut Reader<'_>) -> Option<Bytes> {
    r.forward_tag(b"<")?;

    let mut result = Bytes::new();
    let mut high: Option<u8> = None;

    loop {
        let b = r.read_byte()?;

        if b == b'>' {
            // Odd trailing nibble is padded with 0.
            if let Some(hi) = high {
                result.push(hi << 4);
            }
            return Some(result);
        }

        if is_whitespace(b) {
            continue;
        }

        let nibble = decode_hex_digit(b)?;

        match high {
            None => high = Some(nibble),
            Some(hi) => {
                result.push(hi << 4 | nibble);
                high = None;
            }
        }
    }
}

/// Decode a single ASCII hex digit to its numeric value.
#[inline(always)]
pub(crate) fn decode_hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_hex(input: &[u8]) -> Option<Bytes> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    #[test]
    fn simple() {
        assert_eq!(&*read_hex(b"<48656C6C6F>").unwrap(), b"Hello");
    }

    #[test]
    fn with_whitespace() {
        assert_eq!(&*read_hex(b"<48 65 6C 6C 6F>").unwrap(), b"Hello");
    }

    #[test]
    fn odd_nibble() {
        assert_eq!(&*read_hex(b"<ABC>").unwrap(), &[0xAB, 0xC0]);
    }

    #[test]
    fn empty() {
        assert_eq!(&*read_hex(b"<>").unwrap(), b"");
    }

    #[test]
    fn lowercase() {
        assert_eq!(&*read_hex(b"<abcd>").unwrap(), &[0xAB, 0xCD]);
    }

    #[test]
    fn mixed_case() {
        assert_eq!(&*read_hex(b"<aB3E>").unwrap(), &[0xAB, 0x3E]);
    }
}
