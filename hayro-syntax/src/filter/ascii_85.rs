//! A decoder for ASCII-85-encoded streams.

/// Decode a ASCII-85-encoded stream.
pub fn decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = vec![];

    let mut stream = data
        .iter()
        .cloned()
        .filter(|&b| !matches!(b, b' ' | b'\n' | b'\r' | b'\t'));

    let mut symbols = stream.by_ref().take_while(|&b| b != b'~');

    let (tail_len, tail) = loop {
        match symbols.next() {
            Some(b'z') => decoded.extend_from_slice(&[0; 4]),
            Some(a) => {
                let (b, c, d, e) = match (
                    symbols.next(),
                    symbols.next(),
                    symbols.next(),
                    symbols.next(),
                ) {
                    (Some(b), Some(c), Some(d), Some(e)) => (b, c, d, e),
                    (None, _, _, _) => break (1, [a, b'u', b'u', b'u', b'u']),
                    (Some(b), None, _, _) => break (2, [a, b, b'u', b'u', b'u']),
                    (Some(b), Some(c), None, _) => break (3, [a, b, c, b'u', b'u']),
                    (Some(b), Some(c), Some(d), None) => break (4, [a, b, c, d, b'u']),
                };
                decoded.extend_from_slice(&word_85([a, b, c, d, e])?);
            }
            None => break (0, [b'u'; 5]),
        }
    };

    if tail_len > 0 {
        let last = word_85(tail)?;
        decoded.extend_from_slice(&last[..tail_len - 1]);
    }

    Some(decoded)
}

fn sym_85(byte: u8) -> Option<u8> {
    match byte {
        b @ 0x21..=0x75 => Some(b - 0x21),
        _ => None,
    }
}

fn word_85([a, b, c, d, e]: [u8; 5]) -> Option<[u8; 4]> {
    fn s(b: u8) -> Option<u64> {
        sym_85(b).map(|n| n as u64)
    }
    let (a, b, c, d, e) = (s(a)?, s(b)?, s(c)?, s(d)?, s(e)?);
    let q = (((a * 85 + b) * 85 + c) * 85 + d) * 85 + e;
    // 85^5 > 256^4, the result might not fit in an u32.
    let r = u32::try_from(q).ok()?;
    Some(r.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use crate::filter::ascii_85::decode;

    #[test]
    fn decode_simple() {
        let input = b"87cURDZ~>";
        assert_eq!(decode(input).unwrap(), b"Hello");
    }

    #[test]
    fn decode_spaces() {
        let input = b"87  cURD  Z~>";
        assert_eq!(decode(input).unwrap(), b"Hello");
    }

    #[test]
    fn decode_zeroes() {
        let input = b"z~>";
        assert_eq!(decode(input).unwrap(), [0, 0, 0, 0]);
    }
}
