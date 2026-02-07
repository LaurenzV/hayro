// Keep in sync with `hayro-syntax/src/object/string.rs` (`read_literal`).

use alloc::vec::Vec;

use crate::reader::Reader;

pub(crate) fn decode_into(data: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let mut r = Reader::new(data);

    while let Some(byte) = r.read_byte() {
        match byte {
            b'\\' => {
                let next = r.read_byte()?;

                if is_octal_digit(next) {
                    let second = r.read_byte();
                    let third = r.read_byte();

                    let bytes = match (second, third) {
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

                    let str = core::str::from_utf8(&bytes).unwrap();

                    if let Ok(num) = u8::from_str_radix(str, 8) {
                        out.push(num);
                    }
                } else {
                    match next {
                        b'n' => out.push(0xA),
                        b'r' => out.push(0xD),
                        b't' => out.push(0x9),
                        b'b' => out.push(0x8),
                        b'f' => out.push(0xC),
                        b'(' => out.push(b'('),
                        b')' => out.push(b')'),
                        b'\\' => out.push(b'\\'),
                        b'\n' | b'\r' => {
                            // A conforming reader shall disregard the REVERSE SOLIDUS
                            // and the end-of-line marker following it when reading
                            // the string; the resulting string value shall be
                            // identical to that which would be read if the string
                            // were not split.
                            r.skip_eol();
                        }
                        _ => out.push(next),
                    }
                }
            }
            b'(' | b')' => out.push(byte),
            // An end-of-line marker appearing within a literal string
            // without a preceding REVERSE SOLIDUS shall be treated as
            // a byte value of (0Ah), irrespective of whether the end-of-line
            // marker was a CARRIAGE RETURN (0Dh), a LINE FEED (0Ah), or both.
            b'\n' | b'\r' => {
                out.push(b'\n');
                r.skip_eol();
            }
            other => out.push(other),
        }
    }

    Some(())
}

fn is_octal_digit(byte: u8) -> bool {
    matches!(byte, b'0'..=b'7')
}
