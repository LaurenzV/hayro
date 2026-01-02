//! Comments and white spaces.

use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};

#[inline(always)]
pub(crate) fn is_white_space_character(char: u8) -> bool {
    matches!(char, 0x00 | 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}

#[inline(always)]
pub(crate) fn is_regular_character(char: u8) -> bool {
    match char {
        // Whitespace characters
        0x00 | 0x09 | 0x0a | 0x0c | 0x0d | 0x20 => false,
        // Delimiter characters
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' => false,
        // All other characters are considered regular.
        _ => true,
    }
}

#[inline(always)]
pub(crate) fn is_eol_character(char: u8) -> bool {
    matches!(char, 0x0a | 0x0d)
}

#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub(crate) struct Comment<'a>(pub(crate) &'a [u8]);

impl Skippable for Comment<'_> {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        r.forward_tag(b"%")?;
        r.forward_while(|b| !is_eol_character(b));

        Some(())
    }
}

impl<'a> Readable<'a> for Comment<'a> {
    fn read(r: &mut Reader<'a>, _: &ReaderContext<'_>) -> Option<Self> {
        let bytes = r.skip_in_content_stream::<Comment<'_>>()?;
        let bytes = bytes.get(1..bytes.len()).unwrap();

        Some(Comment(bytes))
    }
}
