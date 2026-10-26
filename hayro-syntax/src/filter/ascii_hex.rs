// Keep in sync with `hayro-postscript/src/string/ascii_hex.rs`.

use crate::trivia::is_white_space_character;
use crate::util::ByteBuf;
use alloc::vec::Vec;

pub(crate) fn decode(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    decode_into(data, limit)
}

/// Decode at most `limit + 1` bytes, so that the caller can tell whether the
/// data decodes to more than `limit` bytes.
pub(crate) fn decode_into<B: ByteBuf>(data: &[u8], limit: usize) -> Option<B> {
    // Find end (at `>` or end of data) and check for whitespace.
    let mut has_whitespace = false;
    let mut end = data.len();

    for (idx, byte) in data.iter().enumerate() {
        has_whitespace |= is_white_space_character(*byte);

        if *byte == b'>' {
            end = idx;
            break;
        }
    }

    let data = &data[..end];
    let bound = limit.saturating_add(1);
    let mut decoded = B::with_capacity(data.len().div_ceil(2).min(bound));

    if !has_whitespace {
        // Fast path, don't need to worry about white spaces.
        let (chunks, remainder) = data.as_chunks::<2>();

        for &[hi, lo] in chunks.iter().take(bound) {
            decoded.push(decode_hex_digit(hi)? << 4 | decode_hex_digit(lo)?);
        }

        if chunks.len() >= bound {
            return Some(decoded);
        }

        if let [hi] = remainder {
            decoded.push(decode_hex_digit(*hi)? << 4);
        }
    } else {
        // Slow path, need to strip white spaces.
        let mut iter = data.iter().copied();
        let mut read_byte = || -> Option<u8> {
            loop {
                let b = iter.next()?;
                if !is_white_space_character(b) {
                    return Some(b);
                }
            }
        };

        for _ in 0..bound {
            match (read_byte(), read_byte()) {
                (Some(hi), Some(lo)) => {
                    decoded.push(decode_hex_digit(hi)? << 4 | decode_hex_digit(lo)?);
                }
                (Some(hi), None) => {
                    decoded.push(decode_hex_digit(hi)? << 4);

                    break;
                }
                (None, _) => break,
            }
        }
    }

    Some(decoded)
}

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
    use crate::filter::ascii_hex::decode;

    #[test]
    fn decode_simple() {
        let input = b"AF3E2901>";
        assert_eq!(
            decode(input, usize::MAX).unwrap(),
            vec![0xaf, 0x3e, 0x29, 0x01]
        );
    }

    #[test]
    fn decode_whitespaces() {
        let input = b"AF3   E2   901>";
        assert_eq!(
            decode(input, usize::MAX).unwrap(),
            vec![0xaf, 0x3e, 0x29, 0x01]
        );
    }

    #[test]
    // Not valid for ASCII hex streams since they require a trailing >,
    // but used by PDF hex strings as well.
    fn decode_without_gt() {
        let input = b"AF3E2901";
        assert_eq!(
            decode(input, usize::MAX).unwrap(),
            vec![0xaf, 0x3e, 0x29, 0x01]
        );
    }

    #[test]
    fn decode_with_padding() {
        let input = b"AF3E291>";
        assert_eq!(
            decode(input, usize::MAX).unwrap(),
            vec![0xaf, 0x3e, 0x29, 0x10]
        );
    }

    #[test]
    fn decode_stops_past_limit() {
        assert_eq!(decode(b"AF3E2901>", 2).unwrap(), vec![0xaf, 0x3e, 0x29]);
        assert_eq!(decode(b"AF 3E 29 01>", 2).unwrap(), vec![0xaf, 0x3e, 0x29]);
        assert_eq!(decode(b"AF3E2>", 2).unwrap(), vec![0xaf, 0x3e, 0x20]);
    }
}
