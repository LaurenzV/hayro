//! Comments and white spaces.

use crate::reader::Reader;
use crate::reader::{Ends, Readable, ReaderContext, ReaderExt, Skippable, Unknown};

const fn build_regular_character_table() -> [bool; 256] {
    let mut table = [true; 256];

    // Whitespace characters.
    table[0x00] = false;
    table[0x09] = false;
    table[0x0a] = false;
    table[0x0c] = false;
    table[0x0d] = false;
    table[0x20] = false;

    // Delimiter characters.
    table[b'(' as usize] = false;
    table[b')' as usize] = false;
    table[b'<' as usize] = false;
    table[b'>' as usize] = false;
    table[b'[' as usize] = false;
    table[b']' as usize] = false;
    table[b'{' as usize] = false;
    table[b'}' as usize] = false;
    table[b'/' as usize] = false;
    table[b'%' as usize] = false;

    table
}

const REGULAR_CHARACTER_TABLE: [bool; 256] = build_regular_character_table();

const fn build_white_space_table() -> [bool; 256] {
    let mut table = [false; 256];

    table[0x00] = true;
    table[0x09] = true;
    table[0x0a] = true;
    table[0x0c] = true;
    table[0x0d] = true;
    table[0x20] = true;

    table
}

const WHITE_SPACE_CHARACTER_TABLE: [bool; 256] = build_white_space_table();

#[inline(always)]
pub(crate) fn is_white_space_character(char: u8) -> bool {
    WHITE_SPACE_CHARACTER_TABLE[char as usize]
}

#[inline(always)]
pub(crate) fn is_regular_character(char: u8) -> bool {
    REGULAR_CHARACTER_TABLE[char as usize]
}

#[inline(always)]
pub(crate) fn is_eol_character(char: u8) -> bool {
    matches!(char, 0x0a | 0x0d)
}

#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub(crate) struct Comment<'a>(pub(crate) &'a [u8]);

impl Skippable for Comment<'_> {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        Self::skip_in(r, &Unknown)
    }
}

impl Comment<'_> {
    #[inline]
    pub(crate) fn skip_in(r: &mut Reader<'_>, ends: &impl Ends) -> Option<()> {
        let start = r.offset();
        r.forward_tag(b"%")?;

        if let Some(end) = ends.comment_end(r.data, start) {
            r.jump(end);
        } else {
            r.forward_while(|b| !is_eol_character(b));
            ends.read(r.offset() - start);
        }

        Some(())
    }
}

impl<'a> Readable<'a> for Comment<'a> {
    fn read(r: &mut Reader<'a>, _: &ReaderContext<'_>) -> Option<Self> {
        let bytes = r.skip::<Comment<'_>>(false)?;
        let bytes = bytes.get(1..bytes.len()).unwrap();

        Some(Comment(bytes))
    }
}
