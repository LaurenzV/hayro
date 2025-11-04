//! Strings.

use crate::crypto::DecryptionTarget;
use crate::filter::ascii_hex::decode_hex_string;
use crate::object::macros::object;
use crate::object::{Object, ObjectLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::is_white_space_character;
use log::warn;
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
// TODO: Make `HexString` and `LiteralString` own their values.

/// A hex-encoded string.
#[derive(Clone, Debug)]
struct HexString<'a>(&'a [u8], bool, ReaderContext<'a>);

impl HexString<'_> {
    /// Returns the content of the string.
    fn get(&self) -> Vec<u8> {
        let decoded = if self.1 {
            let mut cleaned = Vec::with_capacity(self.0.len() + 1);

            for b in self.0.iter().copied() {
                if !is_white_space_character(b) {
                    cleaned.push(b);
                }
            }

            if cleaned.len() % 2 != 0 {
                cleaned.push(b'0');
            }

            // We made sure while parsing that it is a valid hex string.
            decode_hex_string(&cleaned).unwrap()
        } else {
            // We made sure while parsing that it is a valid hex string.
            decode_hex_string(self.0).unwrap()
        };

        if self.2.xref.needs_decryption(&self.2) {
            self.2
                .xref
                .decrypt(
                    self.2.obj_number.unwrap(),
                    &decoded,
                    DecryptionTarget::String,
                )
                .unwrap_or_default()
        } else {
            decoded
        }
    }
}

impl PartialEq for HexString<'_> {
    fn eq(&self, other: &Self) -> bool {
        // TODO: We probably want to ignore escapes.
        self.0 == other.0 && self.1 == other.1
    }
}

impl Skippable for HexString<'_> {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        parse_hex(r).map(|_| {})
    }
}

impl<'a> Readable<'a> for HexString<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let start = r.offset();
        let mut dirty = parse_hex(r)?;
        let end = r.offset();

        // Exclude outer brackets.
        let result = r.range(start + 1..end - 1).unwrap();
        dirty |= !result.len().is_multiple_of(2);

        Some(HexString(result, dirty, ctx.clone()))
    }
}

impl<'a> TryFrom<Object<'a>> for HexString<'a> {
    type Error = ();

    fn try_from(value: Object<'a>) -> Result<Self, Self::Error> {
        match value {
            Object::String(String(InnerString::Hex(h))) => Ok(h),
            _ => Err(()),
        }
    }
}

impl<'a> ObjectLike<'a> for HexString<'a> {}

fn parse_hex(r: &mut Reader<'_>) -> Option<bool> {
    let mut has_whitespace = false;

    r.forward_tag(b"<")?;
    while let Some(b) = r.peek_byte() {
        let is_hex = b.is_ascii_hexdigit();
        let is_whitespace = is_white_space_character(b);
        has_whitespace |= is_whitespace;

        if !is_hex && !is_whitespace {
            break;
        }

        r.read_byte()?;
    }
    r.forward_tag(b">")?;

    Some(has_whitespace)
}

/// A literal string.
#[derive(Debug, Clone)]
struct LiteralString<'a>(&'a [u8], bool, ReaderContext<'a>);

impl<'a> LiteralString<'a> {
    /// Returns the content of the string.
    fn get(&self) -> Cow<'a, [u8]> {
        let decoded = if self.1 {
            let mut cleaned = vec![];
            let mut r = Reader::new(self.0);

            while let Some(byte) = r.read_byte() {
                match byte {
                    b'\\' => {
                        let next = r.read_byte().unwrap();

                        if is_octal_digit(next) {
                            let second = r.read_byte();
                            let third = r.read_byte();

                            let bytes = match (second, third) {
                                (Some(n1), Some(n2)) => {
                                    match (is_octal_digit(n1), is_octal_digit(n2)) {
                                        (true, true) => [next, n1, n2],
                                        (true, _) => {
                                            r.jump(r.offset() - 1);
                                            [b'0', next, n1]
                                        }
                                        _ => {
                                            r.jump(r.offset() - 2);
                                            [b'0', b'0', next]
                                        }
                                    }
                                }
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

                            let str = std::str::from_utf8(&bytes).unwrap();

                            if let Ok(num) = u8::from_str_radix(str, 8) {
                                cleaned.push(num);
                            } else {
                                warn!("overflow occurred while parsing octal literal string");
                            }
                        } else {
                            match next {
                                b'n' => cleaned.push(0xA),
                                b'r' => cleaned.push(0xD),
                                b't' => cleaned.push(0x9),
                                b'b' => cleaned.push(0x8),
                                b'f' => cleaned.push(0xC),
                                b'(' => cleaned.push(b'('),
                                b')' => cleaned.push(b')'),
                                b'\\' => cleaned.push(b'\\'),
                                b'\n' | b'\r' => {
                                    // A conforming reader shall disregard the REVERSE SOLIDUS
                                    // and the end-of-line marker following it when reading
                                    // the string; the resulting string value shall be
                                    // identical to that which would be read if the string
                                    // were not split.
                                    r.skip_eol_characters();
                                }
                                _ => cleaned.push(next),
                            }
                        }
                    }
                    // An end-of-line marker appearing within a literal string
                    // without a preceding REVERSE SOLIDUS shall be treated as
                    // a byte value of (0Ah), irrespective of whether the end-of-line
                    // marker was a CARRIAGE RETURN (0Dh), a LINE FEED (0Ah), or both.
                    b'\n' | b'\r' => {
                        cleaned.push(b'\n');
                        r.skip_eol_characters();
                    }
                    other => cleaned.push(other),
                }
            }

            Cow::Owned(cleaned)
        } else {
            Cow::Borrowed(self.0)
        };

        if self.2.xref.needs_decryption(&self.2) {
            // This might be `None` for example when reading metadata
            // from the trailer dictionary.
            if let Some(obj_number) = self.2.obj_number {
                Cow::Owned(
                    self.2
                        .xref
                        .decrypt(obj_number, &decoded, DecryptionTarget::String)
                        .unwrap_or_default(),
                )
            } else {
                decoded
            }
        } else {
            decoded
        }
    }
}

impl Hash for LiteralString<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
        self.1.hash(state);
    }
}

impl PartialEq for LiteralString<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(other.0) && self.1.eq(&other.1)
    }
}

impl Skippable for LiteralString<'_> {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        parse_literal(r).map(|_| ())
    }
}

impl<'a> Readable<'a> for LiteralString<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let start = r.offset();
        let dirty = parse_literal(r)?;
        let end = r.offset();

        // Exclude outer brackets
        let result = r.range(start + 1..end - 1).unwrap();

        Some(LiteralString(result, dirty, ctx.clone()))
    }
}

impl<'a> TryFrom<Object<'a>> for LiteralString<'a> {
    type Error = ();

    fn try_from(value: Object<'a>) -> Result<Self, Self::Error> {
        match value {
            Object::String(String(InnerString::Literal(l))) => Ok(l),
            _ => Err(()),
        }
    }
}

impl<'a> ObjectLike<'a> for LiteralString<'a> {}

fn parse_literal(r: &mut Reader<'_>) -> Option<bool> {
    r.forward_tag(b"(")?;
    let mut bracket_counter = 1;
    let mut dirty = false;

    while bracket_counter > 0 {
        let byte = r.read_byte()?;

        match byte {
            b'\\' => {
                dirty = true;

                let _ = r.read_byte()?;
            }
            b'(' => bracket_counter += 1,
            b')' => bracket_counter -= 1,
            b'\n' | b'\r' => dirty = true,
            _ => {}
        };
    }

    Some(dirty)
}

#[derive(Clone, Debug, PartialEq)]
enum InnerString<'a> {
    Hex(HexString<'a>),
    Literal(LiteralString<'a>),
}

/// A string.
#[derive(Clone, Debug, PartialEq)]
pub struct String<'a>(InnerString<'a>);

impl<'a> String<'a> {
    /// Returns the content of the string.
    pub fn get(&self) -> Cow<'a, [u8]> {
        match &self.0 {
            InnerString::Hex(hex) => Cow::Owned(hex.get()),
            InnerString::Literal(lit) => lit.get(),
        }
    }
}

impl<'a> From<HexString<'a>> for String<'a> {
    fn from(value: HexString<'a>) -> Self {
        Self(InnerString::Hex(value))
    }
}

impl<'a> From<LiteralString<'a>> for String<'a> {
    fn from(value: LiteralString<'a>) -> Self {
        Self(InnerString::Literal(value))
    }
}

object!(String<'a>, String);

impl Skippable for String<'_> {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        match r.peek_byte()? {
            b'<' => HexString::skip(r, is_content_stream),
            b'(' => LiteralString::skip(r, is_content_stream),
            _ => None,
        }
    }
}

impl<'a> Readable<'a> for String<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let inner = match r.peek_byte()? {
            b'<' => InnerString::Hex(r.read::<HexString>(ctx)?),
            b'(' => InnerString::Literal(r.read::<LiteralString>(ctx)?),
            _ => return None,
        };

        Some(String(inner))
    }
}

fn is_octal_digit(byte: u8) -> bool {
    matches!(byte, b'0'..=b'7')
}

#[cfg(test)]
mod tests {
    use crate::object::string::{HexString, LiteralString, String};
    use crate::reader::Reader;
    use crate::reader::ReaderExt;

    #[test]
    fn hex_string_empty() {
        assert_eq!(
            Reader::new("<>".as_bytes())
                .read_without_context::<HexString>()
                .unwrap()
                .get(),
            vec![]
        );
    }

    #[test]
    fn hex_string_1() {
        assert_eq!(
            Reader::new("<00010203>".as_bytes())
                .read_without_context::<HexString>()
                .unwrap()
                .get(),
            vec![0x00, 0x01, 0x02, 0x03]
        );
    }

    #[test]
    fn hex_string_2() {
        assert_eq!(
            Reader::new("<000102034>".as_bytes())
                .read_without_context::<HexString>()
                .unwrap()
                .get(),
            vec![0x00, 0x01, 0x02, 0x03, 0x40]
        );
    }

    #[test]
    fn hex_string_trailing_1() {
        assert_eq!(
            Reader::new("<000102034>dfgfg4".as_bytes())
                .read_without_context::<HexString>()
                .unwrap()
                .get(),
            vec![0x00, 0x01, 0x02, 0x03, 0x40]
        );
    }

    #[test]
    fn hex_string_trailing_2() {
        assert_eq!(
            Reader::new("<1  3 4>dfgfg4".as_bytes())
                .read_without_context::<HexString>()
                .unwrap()
                .get(),
            vec![0x13, 0x40]
        );
    }

    #[test]
    fn hex_string_trailing_3() {
        assert_eq!(
            Reader::new("<1>dfgfg4".as_bytes())
                .read_without_context::<HexString>()
                .unwrap()
                .get(),
            vec![0x10]
        );
    }

    #[test]
    fn hex_string_invalid_1() {
        assert!(
            Reader::new("<".as_bytes())
                .read_without_context::<HexString>()
                .is_none()
        );
    }

    #[test]
    fn hex_string_invalid_2() {
        assert!(
            Reader::new("34AD".as_bytes())
                .read_without_context::<HexString>()
                .is_none()
        );
    }

    #[test]
    fn literal_string_empty() {
        assert_eq!(
            Reader::new("()".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"".to_vec()
        );
    }

    #[test]
    fn literal_string_1() {
        assert_eq!(
            Reader::new("(Hi there.)".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi there.".to_vec()
        );
    }

    #[test]
    fn literal_string_2() {
        assert!(
            Reader::new("(Hi \\777)".as_bytes())
                .read_without_context::<LiteralString>()
                .is_some()
        );
    }

    #[test]
    fn literal_string_3() {
        assert_eq!(
            Reader::new("(Hi ) there.)".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi ".to_vec()
        );
    }

    #[test]
    fn literal_string_4() {
        assert_eq!(
            Reader::new("(Hi (()) there)".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi (()) there".to_vec()
        );
    }

    #[test]
    fn literal_string_5() {
        assert_eq!(
            Reader::new("(Hi \\()".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi (".to_vec()
        );
    }

    #[test]
    fn literal_string_6() {
        assert_eq!(
            Reader::new("(Hi \\\nthere)".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi there".to_vec()
        );
    }

    #[test]
    fn literal_string_7() {
        assert_eq!(
            Reader::new("(Hi \\05354)".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi +54".to_vec()
        );
    }

    #[test]
    fn literal_string_8() {
        assert_eq!(
            Reader::new("(\\3)".as_bytes())
                .read_without_context::<String>()
                .unwrap()
                .get(),
            b"\x03".to_vec()
        )
    }

    #[test]
    fn literal_string_9() {
        assert_eq!(
            Reader::new("(\\36)".as_bytes())
                .read_without_context::<String>()
                .unwrap()
                .get(),
            b"\x1e".to_vec()
        )
    }

    #[test]
    fn literal_string_10() {
        assert_eq!(
            Reader::new("(\\36ab)".as_bytes())
                .read_without_context::<String>()
                .unwrap()
                .get(),
            b"\x1eab".to_vec()
        )
    }

    #[test]
    fn literal_string_11() {
        assert_eq!(
            Reader::new("(\\00Y)".as_bytes())
                .read_without_context::<String>()
                .unwrap()
                .get(),
            b"\0Y".to_vec()
        )
    }

    #[test]
    fn literal_string_12() {
        assert_eq!(
            Reader::new("(\\0Y)".as_bytes())
                .read_without_context::<String>()
                .unwrap()
                .get(),
            b"\0Y".to_vec()
        )
    }

    #[test]
    fn literal_string_trailing() {
        assert_eq!(
            Reader::new("(Hi there.)abcde".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi there.".to_vec()
        );
    }

    #[test]
    fn literal_string_invalid() {
        assert_eq!(
            Reader::new("(Hi \\778)".as_bytes())
                .read_without_context::<LiteralString>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi \x3F8".to_vec()
        );
    }

    #[test]
    fn string_1() {
        assert_eq!(
            Reader::new("(Hi there.)".as_bytes())
                .read_without_context::<String>()
                .unwrap()
                .get()
                .to_vec(),
            b"Hi there.".to_vec()
        );
    }

    #[test]
    fn string_2() {
        assert_eq!(
            Reader::new("<00010203>".as_bytes())
                .read_without_context::<String>()
                .unwrap()
                .get(),
            vec![0x00, 0x01, 0x02, 0x03]
        );
    }
}
