use crate::trivia::is_white_space_character;
use crate::util::ByteBuf;
use alloc::vec::Vec;

pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
    decode_into(data)
}

pub(crate) fn decode_into<B: ByteBuf>(data: &[u8]) -> Option<B> {
    decode_prefix(data, None)
}

pub(crate) fn decode_prefix<B: ByteBuf>(data: &[u8], truncated: Option<&mut bool>) -> Option<B> {
    let mut incomplete = truncated.as_deref().copied().unwrap_or(false);
    let mut digits: usize = 0;
    let mut last_pair = 0;
    let mut end = 0;

    for (i, &byte) in data.iter().enumerate() {
        if byte.is_ascii_hexdigit() {
            digits += 1;
            end = i + 1;
            if digits.is_multiple_of(2) {
                last_pair = end;
            }
        } else if !is_white_space_character(byte) {
            incomplete = byte != b'>';
            if incomplete && truncated.is_none() {
                return None;
            }
            break;
        }
    }

    if incomplete {
        end = last_pair;
        digits -= digits % 2;
    }
    if let Some(truncated) = truncated {
        *truncated = incomplete;
    }
    let data = &data[..end];
    let mut decoded = B::with_capacity(digits.div_ceil(2));

    if end == digits {
        let (chunks, remainder) = data.as_chunks::<2>();

        for &[hi, lo] in chunks {
            decoded.push(decode_hex_digit(hi)? << 4 | decode_hex_digit(lo)?);
        }

        if let [hi] = remainder {
            decoded.push(decode_hex_digit(*hi)? << 4);
        }
    } else {
        let mut digits = data
            .iter()
            .copied()
            .filter(|&b| !is_white_space_character(b));
        while let Some(hi) = digits.next() {
            let lo = digits.next().unwrap_or(b'0');
            decoded.push(decode_hex_digit(hi)? << 4 | decode_hex_digit(lo)?);
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
        assert_eq!(decode(input).unwrap(), vec![0xaf, 0x3e, 0x29, 0x01]);
    }

    #[test]
    fn decode_whitespaces() {
        let input = b"AF3   E2   901>";
        assert_eq!(decode(input).unwrap(), vec![0xaf, 0x3e, 0x29, 0x01]);
    }

    #[test]
    fn decode_without_gt() {
        let input = b"AF3E2901";
        assert_eq!(decode(input).unwrap(), vec![0xaf, 0x3e, 0x29, 0x01]);
    }

    #[test]
    fn decode_with_padding() {
        let input = b"AF3E291>";
        assert_eq!(decode(input).unwrap(), vec![0xaf, 0x3e, 0x29, 0x10]);
    }

    #[test]
    fn recovered_prefix_capacity() {
        let mut input = b"AB 3 ".to_vec();
        input.extend(core::iter::repeat_n(b' ', 4096));
        input.extend_from_slice(b".45>");
        let mut truncated = false;
        let decoded = super::decode_prefix::<Vec<u8>>(&input, Some(&mut truncated)).unwrap();
        assert!(truncated);
        assert_eq!(decoded, [0xab]);
        assert_eq!(decoded.capacity(), 1);
    }
}
