//! Names.

use crate::filter::ascii_hex::decode_hex_digit;
use crate::object::Object;
use crate::object::macros::object;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, Skippable};
use crate::trivia::is_regular_character;
use core::borrow::Borrow;
use core::fmt::{self, Debug, Formatter};
use core::hash::Hash;
use core::ops::Deref;
use smallvec::SmallVec;

type NameInner = SmallVec<[u8; 31]>;

/// A PDF name object.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(NameInner);

impl Deref for Name {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for Name {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Borrow<[u8]> for Name {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl Name {
    /// Create a new name from a sequence of bytes.
    pub fn new(data: &[u8]) -> Self {
        if !data.contains(&b'#') {
            Self(SmallVec::from_slice(data))
        } else {
            let mut result = SmallVec::new();
            let mut r = Reader::new(data);

            while let Some(b) = r.read_byte() {
                if b == b'#' {
                    // We already verified when skipping that it's a valid hex sequence.
                    let hex = r.read_bytes(2).unwrap();
                    result.push(
                        decode_hex_digit(hex[0]).unwrap() << 4 | decode_hex_digit(hex[1]).unwrap(),
                    );
                } else {
                    result.push(b);
                }
            }

            Self(result)
        }
    }

    /// Return a string representation of the name.
    ///
    /// Returns a placeholder in case the name is not UTF-8 encoded.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).unwrap_or("{non-ascii key}")
    }
}

impl Debug for Name {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match core::str::from_utf8(&self.0) {
            Ok(s) => <str as Debug>::fmt(s, f),
            Err(_) => <[u8] as Debug>::fmt(&self.0, f),
        }
    }
}

object!(Name, Name);

impl Skippable for Name {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        skip_name_like(r, true).map(|_| ())
    }
}

impl Readable<'_> for Name {
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        let start = r.offset();
        skip_name_like(r, true)?;
        let end = r.offset();

        // Exclude leading solidus.
        let data = r.range(start + 1..end)?;
        Some(Self::new(data))
    }
}

// This method is shared by `Name` and the parser for content stream operators (which behave like
// names, except that they aren't preceded by a solidus.
pub(crate) fn skip_name_like(r: &mut Reader<'_>, solidus: bool) -> Option<()> {
    if solidus {
        r.forward_tag(b"/")?;
    }

    let old = r.offset();

    while let Some(b) = r.eat(is_regular_character) {
        if b == b'#' {
            r.eat(|n| n.is_ascii_hexdigit())?;
            r.eat(|n| n.is_ascii_hexdigit())?;
        }
    }

    if !solidus && old == r.offset() {
        return None;
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use crate::object::Name;
    use crate::reader::Reader;
    use crate::reader::ReaderExt;
    use std::ops::Deref;

    #[test]
    fn name_1() {
        assert_eq!(
            Reader::new("/".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b""
        );
    }

    #[test]
    fn name_2() {
        assert!(
            Reader::new("dfg".as_bytes())
                .read_without_context::<Name>()
                .is_none()
        );
    }

    #[test]
    fn name_3() {
        assert!(
            Reader::new("/AB#FG".as_bytes())
                .read_without_context::<Name>()
                .is_none()
        );
    }

    #[test]
    fn name_4() {
        assert_eq!(
            Reader::new("/Name1".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"Name1"
        );
    }

    #[test]
    fn name_5() {
        assert_eq!(
            Reader::new("/ASomewhatLongerName".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"ASomewhatLongerName"
        );
    }

    #[test]
    fn name_6() {
        assert_eq!(
            Reader::new("/A;Name_With-Various***Characters?".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"A;Name_With-Various***Characters?"
        );
    }

    #[test]
    fn name_7() {
        assert_eq!(
            Reader::new("/1.2".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"1.2"
        );
    }

    #[test]
    fn name_8() {
        assert_eq!(
            Reader::new("/$$".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"$$"
        );
    }

    #[test]
    fn name_9() {
        assert_eq!(
            Reader::new("/@pattern".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"@pattern"
        );
    }

    #[test]
    fn name_10() {
        assert_eq!(
            Reader::new("/.notdef".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b".notdef"
        );
    }

    #[test]
    fn name_11() {
        assert_eq!(
            Reader::new("/lime#20Green".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"lime Green"
        );
    }

    #[test]
    fn name_12() {
        assert_eq!(
            Reader::new("/paired#28#29parentheses".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"paired()parentheses"
        );
    }

    #[test]
    fn name_13() {
        assert_eq!(
            Reader::new("/The_Key_of_F#23_Minor".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"The_Key_of_F#_Minor"
        );
    }

    #[test]
    fn name_14() {
        assert_eq!(
            Reader::new("/A#42".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"AB"
        );
    }

    #[test]
    fn name_15() {
        assert_eq!(
            Reader::new("/A#3b".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"A;"
        );
    }

    #[test]
    fn name_16() {
        assert_eq!(
            Reader::new("/A#3B".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"A;"
        );
    }

    #[test]
    fn name_17() {
        assert_eq!(
            Reader::new("/k1  ".as_bytes())
                .read_without_context::<Name>()
                .unwrap()
                .deref(),
            b"k1"
        );
    }
}
