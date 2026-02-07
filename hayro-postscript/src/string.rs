//! Literal string `(...)` parsing.

use crate::object::Bytes;
use crate::reader::Reader;

/// Read a literal string `(...)`, returning the decoded contents.
///
/// The reader must be positioned at the opening `(`.
pub(crate) fn read(r: &mut Reader<'_>) -> Option<Bytes> {
    // Skip the inner content to find the matching `)`, handling nested parens.
    let start = r.offset();
    skip(r)?;
    let end = r.offset();

    // Exclude outer parentheses.
    let data = r.range(start + 1..end - 1)?;

    let mut inner = Reader::new(data);
    let mut result = Bytes::new();

    while let Some(byte) = inner.read_byte() {
        match byte {
            b'\\' => {
                let next = inner.read_byte()?;

                if is_octal_digit(next) {
                    let second = inner.read_byte();
                    let third = inner.read_byte();

                    let digits = match (second, third) {
                        (Some(n1), Some(n2)) => match (is_octal_digit(n1), is_octal_digit(n2)) {
                            (true, true) => [next, n1, n2],
                            (true, _) => {
                                inner.jump(inner.offset() - 1);
                                [b'0', next, n1]
                            }
                            _ => {
                                inner.jump(inner.offset() - 2);
                                [b'0', b'0', next]
                            }
                        },
                        (Some(n1), None) => {
                            if is_octal_digit(n1) {
                                [b'0', next, n1]
                            } else {
                                inner.jump(inner.offset() - 1);
                                [b'0', b'0', next]
                            }
                        }
                        _ => [b'0', b'0', next],
                    };

                    let s = core::str::from_utf8(&digits).unwrap();
                    // Octal values that overflow u8 are implementation-defined;
                    // we truncate to the low 8 bits as hayro-syntax does.
                    if let Ok(num) = u8::from_str_radix(s, 8) {
                        result.push(num);
                    }
                } else {
                    match next {
                        b'n' => result.push(0x0A),
                        b'r' => result.push(0x0D),
                        b't' => result.push(0x09),
                        b'b' => result.push(0x08),
                        b'f' => result.push(0x0C),
                        b'(' => result.push(b'('),
                        b')' => result.push(b')'),
                        b'\\' => result.push(b'\\'),
                        // Line continuation: backslash followed by EOL is discarded.
                        b'\n' | b'\r' => {
                            inner.skip_eol();
                        }
                        // Unknown escape: the spec says to ignore the backslash.
                        _ => result.push(next),
                    }
                }
            }
            // Balanced parens are kept literally.
            b'(' | b')' => result.push(byte),
            // Bare EOL normalised to LF.
            b'\n' | b'\r' => {
                result.push(b'\n');
                inner.skip_eol();
            }
            other => result.push(other),
        }
    }

    Some(result)
}

/// Skip past a literal string without decoding.
fn skip(r: &mut Reader<'_>) -> Option<()> {
    r.forward_tag(b"(")?;
    let mut depth = 1u32;

    while depth > 0 {
        let byte = r.read_byte()?;
        match byte {
            b'\\' => {
                let _ = r.read_byte()?;
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
    }

    Some(())
}

fn is_octal_digit(b: u8) -> bool {
    matches!(b, b'0'..=b'7')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_string(input: &[u8]) -> Option<Bytes> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    #[test]
    fn empty() {
        assert_eq!(&*read_string(b"()").unwrap(), b"");
    }

    #[test]
    fn simple() {
        assert_eq!(&*read_string(b"(Hello)").unwrap(), b"Hello");
    }

    #[test]
    fn nested_parens() {
        assert_eq!(
            &*read_string(b"(Hi (()) there)").unwrap(),
            b"Hi (()) there"
        );
    }

    #[test]
    fn escape_n() {
        assert_eq!(&*read_string(b"(a\\nb)").unwrap(), b"a\nb");
    }

    #[test]
    fn escape_r() {
        assert_eq!(&*read_string(b"(a\\rb)").unwrap(), b"a\rb");
    }

    #[test]
    fn escape_t() {
        assert_eq!(&*read_string(b"(a\\tb)").unwrap(), b"a\tb");
    }

    #[test]
    fn escape_b() {
        assert_eq!(&*read_string(b"(a\\bb)").unwrap(), &[b'a', 0x08, b'b']);
    }

    #[test]
    fn escape_f() {
        assert_eq!(&*read_string(b"(a\\fb)").unwrap(), &[b'a', 0x0C, b'b']);
    }

    #[test]
    fn escape_backslash() {
        assert_eq!(&*read_string(b"(a\\\\b)").unwrap(), b"a\\b");
    }

    #[test]
    fn escape_parens() {
        assert_eq!(&*read_string(b"(Hi \\()").unwrap(), b"Hi (");
    }

    #[test]
    fn octal_three_digits() {
        assert_eq!(&*read_string(b"(\\053)").unwrap(), b"+");
    }

    #[test]
    fn octal_two_digits() {
        assert_eq!(&*read_string(b"(\\36)").unwrap(), b"\x1e");
    }

    #[test]
    fn octal_one_digit() {
        assert_eq!(&*read_string(b"(\\3)").unwrap(), b"\x03");
    }

    #[test]
    fn line_continuation_lf() {
        assert_eq!(&*read_string(b"(Hi \\\nthere)").unwrap(), b"Hi there");
    }

    #[test]
    fn line_continuation_cr() {
        assert_eq!(&*read_string(b"(Hi \\\rthere)").unwrap(), b"Hi there");
    }

    #[test]
    fn line_continuation_crlf() {
        assert_eq!(&*read_string(b"(Hi \\\r\nthere)").unwrap(), b"Hi there");
    }

    #[test]
    fn bare_eol_lf() {
        assert_eq!(&*read_string(b"(a\nb)").unwrap(), b"a\nb");
    }

    #[test]
    fn bare_eol_cr() {
        assert_eq!(&*read_string(b"(a\rb)").unwrap(), b"a\nb");
    }

    #[test]
    fn bare_eol_crlf() {
        assert_eq!(&*read_string(b"(a\r\nb)").unwrap(), b"a\nb");
    }
}
