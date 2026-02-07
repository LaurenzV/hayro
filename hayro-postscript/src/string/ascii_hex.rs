// Keep in sync with `hayro-syntax/src/filter/ascii_hex.rs`.

use crate::reader::is_whitespace;
use alloc::vec::Vec;

pub(crate) fn decode_into(data: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let has_whitespace = data.iter().any(|&b| is_whitespace(b));

    out.reserve(data.len().div_ceil(2));

    if !has_whitespace {
        // Fast path, don't need to worry about white spaces.
        let mut i = 0;
        while i + 1 < data.len() {
            out.push(decode_hex_digit(data[i])? << 4 | decode_hex_digit(data[i + 1])?);
            i += 2;
        }
        if i < data.len() {
            out.push(decode_hex_digit(data[i])? << 4);
        }
    } else {
        // Slow path, need to strip white spaces.
        let mut iter = data.iter().copied();
        let mut read_byte = || -> Option<u8> {
            loop {
                let b = iter.next()?;
                if !is_whitespace(b) {
                    return Some(b);
                }
            }
        };

        loop {
            match (read_byte(), read_byte()) {
                (Some(hi), Some(lo)) => {
                    out.push(decode_hex_digit(hi)? << 4 | decode_hex_digit(lo)?);
                }
                (Some(hi), None) => {
                    out.push(decode_hex_digit(hi)? << 4);

                    break;
                }
                (None, _) => break,
            }
        }
    }

    Some(())
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
