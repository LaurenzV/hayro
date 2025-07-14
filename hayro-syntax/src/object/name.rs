//! Name objects.

use crate::object::Object;
use crate::object::macros::object;
use crate::reader::{Readable, Reader, ReaderContext, Skippable};
use crate::trivia::is_regular_character;
use std::fmt::Debug;
use std::hash::Hash;
use std::ops::Deref;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Cow<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
}

/// A PDF name.
#[derive(Debug, Clone)]
pub struct Name<'a>(Cow<'a>);

// Two names should be equal even if one is borrowed and the other is owned,
// so we need these manual implementations.

impl<'a> Hash for Name<'a> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl PartialEq for Name<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl Eq for Name<'_> {}

impl PartialOrd for Name<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.deref().partial_cmp(other.deref())
    }
}

impl Ord for Name<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl<'a> AsRef<Name<'a>> for Name<'a> {
    fn as_ref(&self) -> &Name<'a> {
        self
    }
}

impl Deref for Name<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match &self.0 {
            Cow::Borrowed(a) => a,
            Cow::Owned(v) => v.as_ref(),
        }
    }
}

impl<'a> Name<'a> {
    /// Create a new name from a sequence of bytes.
    pub fn new(data: &'a [u8]) -> Name<'a> {
        fn convert_hex(c: u8) -> u8 {
            match c {
                b'A'..=b'F' => c - b'A' + 10,
                b'a'..=b'f' => c - b'a' + 10,
                b'0'..=b'9' => c - b'0',
                _ => unreachable!(),
            }
        }

        let data = if !data.iter().any(|c| *c == b'#') {
            Cow::Borrowed(data)
        } else {
            let mut cleaned = vec![];

            let mut r = Reader::new(data);

            while let Some(b) = r.read_byte() {
                if b == b'#' {
                    // We already verified when skipping that it's a valid hex sequence.
                    let hex = r.read_bytes(2).unwrap();
                    cleaned.push(convert_hex(hex[0]) << 4 | convert_hex(hex[1]));
                } else {
                    cleaned.push(b);
                }
            }

            Cow::Owned(cleaned)
        };

        Self(data)
    }

    /// Create a new name from an unescape bytes string.
    pub(crate) const fn from_unescaped(data: &'a [u8]) -> Name<'a> {
        Self(Cow::Borrowed(data))
    }

    /// Return a string representation of the name.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.deref()).unwrap_or("{non-ascii key}")
    }
}

object!(Name<'a>, Name);

impl Skippable for Name<'_> {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        skip_name_like(r, true).map(|_| ())
    }
}

impl<'a> Readable<'a> for Name<'a> {
    fn read(r: &mut Reader<'a>, _: ReaderContext) -> Option<Self> {
        let data = {
            let start = r.offset();
            skip_name_like(r, true)?;
            let end = r.offset();

            r.range(start + 1..end).unwrap()
        };

        Some(Self::new(data))
    }
}

// This method is shared by `Name` and the parser for content stream operators (which behave like
// names, except that they aren't preceded by a solidus.
pub(crate) fn skip_name_like(r: &mut Reader, solidus: bool) -> Option<()> {
    if solidus {
        r.forward_tag(b"/")?;
    }

    let mut old = r.offset();

    while let Some(b) = r.eat(|n| is_regular_character(n)) {
        match b {
            b'#' => {
                r.eat(|n| n.is_ascii_hexdigit())?;
                r.eat(|n| n.is_ascii_hexdigit())?;
            }
            _ => {}
        }
    }

    if !solidus && old == r.offset() {
        return None;
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use crate::object::name::Name;
    use crate::reader::Reader;
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
