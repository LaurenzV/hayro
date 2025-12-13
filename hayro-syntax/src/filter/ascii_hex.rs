use crate::reader::Reader;
use crate::trivia::is_white_space_character;

pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut end = 0;
    let mut needs_cleaning = false;

    let mut reader = Reader::new(data);

    // We are lenient and don't require a > in the stream.
    while let Some(byte) = reader.read_byte() {
        match byte {
            b'>' => {
                if end % 2 != 0 {
                    needs_cleaning = true;
                }

                break;
            }
            b if b.is_ascii_hexdigit() => {}
            b if is_white_space_character(b) => {
                needs_cleaning = true;
            }
            _ => {
                return None;
            }
        }

        end += 1;
    }

    let trimmed = &data[0..end];

    if needs_cleaning {
        let cleaned = trimmed
            .iter()
            .flat_map(|c| {
                if c.is_ascii_hexdigit() {
                    Some(*c)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        decode_hex_string(&cleaned).ok()
    } else {
        decode_hex_string(trimmed).ok()
    }
}

// Taken from the `hex` crate

pub(crate) fn decode_hex_string(str: &[u8]) -> Result<Vec<u8>, ()> {
    str.chunks(2)
        // In case length is not a multiple of 2, pad with 0.
        .map(|pair| Ok(val(pair[0])? << 4 | val(*pair.get(1).unwrap_or(&b'0'))?))
        .collect()
}

fn val(c: u8) -> Result<u8, ()> {
    match c {
        b'A'..=b'F' => Ok(c - b'A' + 10),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'0'..=b'9' => Ok(c - b'0'),
        _ => Err(()),
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
    // Technically not valid, but doesn't hurt to support either way.
    fn decode_without_gt() {
        let input = b"AF3E2901";
        assert_eq!(decode(input).unwrap(), vec![0xaf, 0x3e, 0x29, 0x01]);
    }

    #[test]
    fn decode_with_padding() {
        let input = b"AF3E291>";
        assert_eq!(decode(input).unwrap(), vec![0xaf, 0x3e, 0x29, 0x10]);
    }
}
