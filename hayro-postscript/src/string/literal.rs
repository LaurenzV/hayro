// Keep in sync with `hayro-syntax/src/object/string.rs`.

use alloc::vec::Vec;

use crate::reader::Reader;

/// Decode a literal string's inner bytes (escape sequences, octal, EOL
/// normalization) and append to `out`.
pub(crate) fn decode_into(data: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let mut r = Reader::new(data);

    while let Some(byte) = r.read_byte() {
        match byte {
            b'\\' => {
                let next = r.read_byte()?;

                if is_octal_digit(next) {
                    let second = r.read_byte();
                    let third = r.read_byte();

                    let digits = match (second, third) {
                        (Some(n1), Some(n2)) => match (is_octal_digit(n1), is_octal_digit(n2)) {
                            (true, true) => [next, n1, n2],
                            (true, _) => {
                                r.jump(r.offset() - 1);
                                [b'0', next, n1]
                            }
                            _ => {
                                r.jump(r.offset() - 2);
                                [b'0', b'0', next]
                            }
                        },
                        (Some(n1), None) => {
                            if is_octal_digit(n1) {
                                [b'0', next, n1]
                            } else {
                                r.jump(r.offset() - 1);
                                [b'0', b'0', next]
                            }
                        }
                        _ => [b'0', b'0', next],
                    };

                    let s = core::str::from_utf8(&digits).unwrap();
                    if let Ok(num) = u8::from_str_radix(s, 8) {
                        out.push(num);
                    }
                } else {
                    match next {
                        b'n' => out.push(0x0A),
                        b'r' => out.push(0x0D),
                        b't' => out.push(0x09),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'(' => out.push(b'('),
                        b')' => out.push(b')'),
                        b'\\' => out.push(b'\\'),
                        // Line continuation: backslash followed by EOL is discarded.
                        b'\n' | b'\r' => {
                            r.skip_eol();
                        }
                        // Unknown escape: the spec says to ignore the backslash.
                        _ => out.push(next),
                    }
                }
            }
            // Balanced parens are kept literally.
            b'(' | b')' => out.push(byte),
            // Bare EOL normalised to LF.
            b'\n' | b'\r' => {
                out.push(b'\n');
                r.skip_eol();
            }
            other => out.push(other),
        }
    }

    Some(())
}

fn is_octal_digit(b: u8) -> bool {
    matches!(b, b'0'..=b'7')
}
