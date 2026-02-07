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
                            // "If the \ is followed immediately by a newline
                            // (CR, LF, or CR-LF pair), the scanner ignores
                            // both the initial \ and the newline; this breaks
                            // a string into multiple lines without including
                            // the newline character as part of the string."
                            r.skip_eol();
                        }
                        _ => out.push(next),
                    }
                }
            }
            b'(' | b')' => out.push(byte),
            // "But if a newline appears without a preceding \, the result is
            // equivalent to \n."
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
