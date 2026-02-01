//! Strings.

use crate::crypto::DecryptionTarget;
use crate::filter::ascii_hex;
use crate::object::Object;
use crate::object::macros::object;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::is_white_space_character;
use core::ops::Deref;
use log::warn;
use smallvec::SmallVec;

type StringInner = SmallVec<[u8; 23]>;

/// A PDF string object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct String(StringInner);

impl String {
    /// Returns the string data as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for String {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for String {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

object!(String, String);

impl Skippable for String {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        match r.peek_byte()? {
            b'<' => skip_hex(r),
            b'(' => skip_literal(r),
            _ => None,
        }
    }
}

impl Readable<'_> for String {
    fn read(r: &mut Reader<'_>, ctx: &ReaderContext<'_>) -> Option<Self> {
        let decoded = match r.peek_byte()? {
            b'<' => read_hex(r)?,
            b'(' => read_literal(r)?,
            _ => return None,
        };

        // Apply decryption if needed.
        let final_data = if ctx.xref().needs_decryption(ctx) {
            if let Some(obj_number) = ctx.obj_number() {
                ctx.xref()
                    .decrypt(obj_number, &decoded, DecryptionTarget::String)
                    .map(SmallVec::from_vec)
                    .unwrap_or(decoded)
            } else {
                decoded
            }
        } else {
            decoded
        };

        Some(Self(final_data))
    }
}

fn skip_hex(r: &mut Reader<'_>) -> Option<()> {
    r.forward_tag(b"<")?;
    while let Some(b) = r.peek_byte() {
        let is_hex = b.is_ascii_hexdigit();
        let is_whitespace = is_white_space_character(b);

        if !is_hex && !is_whitespace {
            break;
        }

        r.read_byte()?;
    }
    r.forward_tag(b">")?;

    Some(())
}

fn read_hex(r: &mut Reader<'_>) -> Option<StringInner> {
    let start = r.offset();
    skip_hex(r)?;
    let end = r.offset();

    // Exclude outer brackets.
    let raw = r.range(start + 1..end - 1)?;
    let decoded = ascii_hex::decode(raw)?;

    Some(SmallVec::from_vec(decoded))
}

fn skip_literal(r: &mut Reader<'_>) -> Option<()> {
    r.forward_tag(b"(")?;
    let mut bracket_counter = 1;

    while bracket_counter > 0 {
        let byte = r.read_byte()?;

        match byte {
            b'\\' => {
                let _ = r.read_byte()?;
            }
            b'(' => bracket_counter += 1,
            b')' => bracket_counter -= 1,
            _ => {}
        };
    }

    Some(())
}

fn read_literal(r: &mut Reader<'_>) -> Option<StringInner> {
    let start = r.offset();
    skip_literal(r)?;
    let end = r.offset();

    // Exclude outer parentheses.
    let data = r.range(start + 1..end - 1)?;

    let mut r = Reader::new(data);
    let mut result = SmallVec::new();

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
                        result.push(num);
                    } else {
                        warn!("overflow occurred while parsing octal literal string");
                    }
                } else {
                    match next {
                        b'n' => result.push(0xA),
                        b'r' => result.push(0xD),
                        b't' => result.push(0x9),
                        b'b' => result.push(0x8),
                        b'f' => result.push(0xC),
                        b'(' => result.push(b'('),
                        b')' => result.push(b')'),
                        b'\\' => result.push(b'\\'),
                        b'\n' | b'\r' => {
                            // A conforming reader shall disregard the REVERSE SOLIDUS
                            // and the end-of-line marker following it when reading
                            // the string; the resulting string value shall be
                            // identical to that which would be read if the string
                            // were not split.
                            r.skip_eol_characters();
                        }
                        _ => result.push(next),
                    }
                }
            }
            b'(' | b')' => result.push(byte),
            // An end-of-line marker appearing within a literal string
            // without a preceding REVERSE SOLIDUS shall be treated as
            // a byte value of (0Ah), irrespective of whether the end-of-line
            // marker was a CARRIAGE RETURN (0Dh), a LINE FEED (0Ah), or both.
            b'\n' | b'\r' => {
                result.push(b'\n');
                r.skip_eol_characters();
            }
            other => result.push(other),
        }
    }

    Some(result)
}

fn is_octal_digit(byte: u8) -> bool {
    matches!(byte, b'0'..=b'7')
}

#[cfg(test)]
mod tests {
    use crate::object::String;
    use crate::reader::Reader;
    use crate::reader::ReaderExt;

    #[test]
    fn hex_string_empty() {
        assert_eq!(
            Reader::new(b"<>")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b""
        );
    }

    #[test]
    fn hex_string_1() {
        assert_eq!(
            Reader::new(b"<00010203>")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            &[0x00, 0x01, 0x02, 0x03]
        );
    }

    #[test]
    fn hex_string_2() {
        assert_eq!(
            Reader::new(b"<000102034>")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            &[0x00, 0x01, 0x02, 0x03, 0x40]
        );
    }

    #[test]
    fn hex_string_trailing_1() {
        assert_eq!(
            Reader::new(b"<000102034>dfgfg4")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            &[0x00, 0x01, 0x02, 0x03, 0x40]
        );
    }

    #[test]
    fn hex_string_trailing_2() {
        assert_eq!(
            Reader::new(b"<1  3 4>dfgfg4")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            &[0x13, 0x40]
        );
    }

    #[test]
    fn hex_string_trailing_3() {
        assert_eq!(
            Reader::new(b"<1>dfgfg4")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            &[0x10]
        );
    }

    #[test]
    fn hex_string_invalid_1() {
        assert!(Reader::new(b"<").read_without_context::<String>().is_none());
    }

    #[test]
    fn hex_string_invalid_2() {
        assert!(
            Reader::new(b"34AD")
                .read_without_context::<String>()
                .is_none()
        );
    }

    #[test]
    fn literal_string_empty() {
        assert_eq!(
            Reader::new(b"()")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b""
        );
    }

    #[test]
    fn literal_string_1() {
        assert_eq!(
            Reader::new(b"(Hi there.)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi there."
        );
    }

    #[test]
    fn literal_string_2() {
        assert!(
            Reader::new(b"(Hi \\777)")
                .read_without_context::<String>()
                .is_some()
        );
    }

    #[test]
    fn literal_string_3() {
        assert_eq!(
            Reader::new(b"(Hi ) there.)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi "
        );
    }

    #[test]
    fn literal_string_4() {
        assert_eq!(
            Reader::new(b"(Hi (()) there)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi (()) there"
        );
    }

    #[test]
    fn literal_string_5() {
        assert_eq!(
            Reader::new(b"(Hi \\()")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi ("
        );
    }

    #[test]
    fn literal_string_6() {
        assert_eq!(
            Reader::new(b"(Hi \\\nthere)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi there"
        );
    }

    #[test]
    fn literal_string_7() {
        assert_eq!(
            Reader::new(b"(Hi \\05354)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi +54"
        );
    }

    #[test]
    fn literal_string_8() {
        assert_eq!(
            Reader::new(b"(\\3)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"\x03"
        );
    }

    #[test]
    fn literal_string_9() {
        assert_eq!(
            Reader::new(b"(\\36)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"\x1e"
        );
    }

    #[test]
    fn literal_string_10() {
        assert_eq!(
            Reader::new(b"(\\36ab)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"\x1eab"
        );
    }

    #[test]
    fn literal_string_11() {
        assert_eq!(
            Reader::new(b"(\\00Y)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"\0Y"
        );
    }

    #[test]
    fn literal_string_12() {
        assert_eq!(
            Reader::new(b"(\\0Y)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"\0Y"
        );
    }

    #[test]
    fn literal_string_trailing() {
        assert_eq!(
            Reader::new(b"(Hi there.)abcde")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi there."
        );
    }

    #[test]
    fn literal_string_invalid() {
        assert_eq!(
            Reader::new(b"(Hi \\778)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi \x3F8"
        );
    }

    #[test]
    fn string_1() {
        assert_eq!(
            Reader::new(b"(Hi there.)")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            b"Hi there."
        );
    }

    #[test]
    fn string_2() {
        assert_eq!(
            Reader::new(b"<00010203>")
                .read_without_context::<String>()
                .unwrap()
                .as_bytes(),
            &[0x00, 0x01, 0x02, 0x03]
        );
    }
}
