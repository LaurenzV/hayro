/*!
A parser for `CMap` files, as they are found in PDFs.

This crate provides a parser for `CMap` files and allows you to
- Map character codes from text-showing operators to CID identifiers.
- Map CIDs to Unicode characters or strings.

## Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

mod parse;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// The name of a `CMap`.
pub type CMapName<'a> = &'a [u8];
/// A CID (Character Identifier).
pub type Cid = u32;

/// Let's limit the number of nested `usecmap` references to 16.
const MAX_NESTING_DEPTH: u32 = 16;

/// A parsed `CMap`.
#[derive(Debug, Clone)]
pub struct CMap {
    metadata: Metadata,
    codespace_ranges: Vec<CodespaceRange>,
    cid_ranges: Vec<CidRange>,
    notdef_ranges: Vec<CidRange>,
    bf_entries: Vec<BfRange>,
    base: Option<Box<Self>>,
}

impl CMap {
    /// Parse a `CMap` from raw bytes.
    ///
    /// The `get_cmap` callback is used to recursively resolve `CMaps` that
    /// are referenced via `usecmap`.
    pub fn parse<'a>(
        data: &[u8],
        get_cmap: impl Fn(CMapName<'_>) -> Option<&'a [u8]> + Clone + 'a,
    ) -> Option<Self> {
        parse::parse(data, get_cmap, 0)
    }

    /// Create an Identity-H `CMap`.
    pub fn identity_h() -> Self {
        Self::identity(WritingMode::Horizontal, b"Identity-H")
    }

    /// Create an Identity-V `CMap`.
    pub fn identity_v() -> Self {
        Self::identity(WritingMode::Vertical, b"Identity-V")
    }

    fn identity(writing_mode: WritingMode, name: &[u8]) -> Self {
        Self {
            metadata: Metadata {
                character_collection: Some(CharacterCollection {
                    registry: Vec::from(b"Adobe" as &[u8]),
                    ordering: Vec::from(b"Identity" as &[u8]),
                    supplement: 0,
                }),
                name: Vec::from(name),
                writing_mode,
            },
            codespace_ranges: vec![CodespaceRange {
                number_bytes: 2,
                low: 0,
                high: 0xFFFF,
            }],
            cid_ranges: vec![CidRange {
                range: Range {
                    start: 0,
                    end: 0xFFFF,
                },
                cid_start: 0,
            }],
            notdef_ranges: Vec::new(),
            bf_entries: Vec::new(),
            base: None,
        }
    }

    /// Return the metadata of this `CMap`.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Look up the CID code of a character code.
    ///
    /// Returns `None` if the code is not within any codespace range for the
    /// given byte length.
    pub fn lookup_cid_code(&self, code: u32, byte_len: u8) -> Option<Cid> {
        let in_codespace = self
            .codespace_ranges
            .iter()
            .any(|r| r.number_bytes == byte_len && code >= r.low && code <= r.high);

        if !in_codespace {
            return None;
        }

        if let Some(entry) = find_in_ranges(&self.cid_ranges, code) {
            let offset = code.checked_sub(entry.range.start)?;
            return entry.cid_start.checked_add(offset);
        } else if let Some(entry) = find_in_ranges(&self.notdef_ranges, code) {
            // For `.notdef` ranges, all codes map to the same `.notdef` CID, so
            // no adding of the offset here.
            return Some(entry.cid_start);
        }

        // If character code is in code space range but has no active mapping, so
        // assume `.notdef`.
        Some(
            self.base
                .as_ref()
                .and_then(|b| b.lookup_cid_code(code, byte_len))
                .unwrap_or(0),
        )
    }

    /// Look up the base font code of the given character code. This is usually
    /// used for `ToUnicode` `CMaps`
    ///
    /// Returns `None` if no mapping is available.
    pub fn lookup_unicode_code(&self, code: u32) -> Option<UnicodeString> {
        if let Some(entry) = find_in_ranges(&self.bf_entries, code) {
            let offset = u16::try_from(code - entry.range.start).ok()?;

            fn decode_utf16(units: &[u16]) -> Option<UnicodeString> {
                let mut iter = core::char::decode_utf16(units.iter().copied());
                let first = iter.next()?.ok()?;

                if iter.next().is_none() {
                    Some(UnicodeString::Char(first))
                } else {
                    let s = String::from_utf16(units).ok()?;
                    Some(UnicodeString::String(s))
                }
            }

            return if offset == 0 {
                Some(decode_utf16(&entry.dst_base)?)
            } else {
                let mut units = entry.dst_base.clone();
                *units.last_mut()? = units.last()?.checked_add(offset)?;
                Some(decode_utf16(&units)?)
            };
        }

        self.base.as_ref()?.lookup_unicode_code(code)
    }
}

trait HasRange {
    fn range(&self) -> &Range;
}

fn find_in_ranges<T: HasRange>(entries: &[T], code: u32) -> Option<&T> {
    let idx = entries
        .binary_search_by(|entry| {
            let r = entry.range();
            if code < r.start {
                core::cmp::Ordering::Greater
            } else if code > r.end {
                core::cmp::Ordering::Less
            } else {
                core::cmp::Ordering::Equal
            }
        })
        .ok()?;

    Some(&entries[idx])
}

/// A range with a start and end code.
#[derive(Debug, Clone)]
pub(crate) struct Range {
    pub(crate) start: u32,
    pub(crate) end: u32,
}

/// A range of character codes mapped to CIDs.
#[derive(Debug, Clone)]
pub struct CidRange {
    pub(crate) range: Range,
    pub(crate) cid_start: Cid,
}

impl HasRange for CidRange {
    fn range(&self) -> &Range {
        &self.range
    }
}

/// A character code to Unicode mapping (potentially a range).
#[derive(Debug, Clone)]
pub(crate) struct BfRange {
    pub(crate) range: Range,
    /// UTF-16 code units. For ranges, the last unit is incremented by the offset.
    pub(crate) dst_base: Vec<u16>,
}

impl HasRange for BfRange {
    fn range(&self) -> &Range {
        &self.range
    }
}

/// A codespace range defining valid character code byte sequences.
#[derive(Debug, Clone)]
pub(crate) struct CodespaceRange {
    pub(crate) number_bytes: u8,
    pub(crate) low: u32,
    pub(crate) high: u32,
}

/// A Unicode value decoded from a `CMap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnicodeString {
    /// A single Unicode character.
    Char(char),
    /// A string consisting of multiple Unicode characters, stored as a UTF-8 string.
    String(String),
}

/// Metadata extracted from a `CMap` file.
#[derive(Debug, Clone)]
pub struct Metadata {
    /// The referenced character collection.
    pub character_collection: Option<CharacterCollection>,
    /// The `CMap` name.
    pub name: Vec<u8>,
    /// The writing mode.
    pub writing_mode: WritingMode,
}

/// A CID character collection identifying the character set and ordering.
#[derive(Debug, Clone)]
pub struct CharacterCollection {
    /// The registry name (e.g. `b"Adobe"`).
    pub registry: Vec<u8>,
    /// The ordering name (e.g. `b"Japan1"`).
    pub ordering: Vec<u8>,
    /// The supplement number.
    pub supplement: i32,
}

/// The writing mode of a `CMap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    /// Horizontal writing mode.
    #[default]
    Horizontal,
    /// Vertical writing mode.
    Vertical,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note that those CMaps might not be completely valid according to the rules
    // of CMap/Postscript, but since our parser is very lenient and doesn't run a real
    // interpreter we can shorten them by a lot.

    const PREAMBLE: &[u8] = br#"/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 0 def
end def
/CMapName /Test def
/WMode 0 def
2 begincodespacerange
<00> <FF>
<0000> <FFFF>
endcodespacerange
"#;

    fn parse_with_preamble(body: &[u8]) -> CMap {
        let mut data = Vec::new();
        data.extend_from_slice(PREAMBLE);
        data.extend_from_slice(body);
        CMap::parse(&data, |_| None).unwrap()
    }

    #[test]
    fn metadata_parsing() {
        let data = br#"
/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 6 def
end def
/CMapName /Adobe-Japan1-H def
/CMapType 1 def
/WMode 0 def
endcmap"#;

        let cmap = CMap::parse(data, |_| None).unwrap();
        let cc = cmap.metadata().character_collection.as_ref().unwrap();
        assert_eq!(cc.registry, b"Adobe");
        assert_eq!(cc.ordering, b"Japan1");
        assert_eq!(cc.supplement, 6);
        assert_eq!(cmap.metadata().name, b"Adobe-Japan1-H");
        assert_eq!(cmap.metadata().writing_mode, WritingMode::Horizontal);
    }

    #[test]
    fn vertical_writing_mode() {
        let data = br#"
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 6 def
end def
/CMapName /Adobe-Japan1-V def
/WMode 1 def
"#;

        let cmap = CMap::parse(data, |_| None).unwrap();
        assert_eq!(cmap.metadata().writing_mode, WritingMode::Vertical);
        assert_eq!(cmap.metadata().name, b"Adobe-Japan1-V");
    }

    #[test]
    fn cid_range_lookup() {
        let cmap = parse_with_preamble(
            br#"
3 begincidrange
<0000> <00FF> 0
<0100> <01FF> 256
<8140> <817E> 633
endcidrange
"#,
        );

        assert_eq!(cmap.lookup_cid_code(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x0042, 2), Some(0x42));
        assert_eq!(cmap.lookup_cid_code(0x00FF, 2), Some(0xFF));

        assert_eq!(cmap.lookup_cid_code(0x0100, 2), Some(256));
        assert_eq!(cmap.lookup_cid_code(0x01FF, 2), Some(511));

        assert_eq!(cmap.lookup_cid_code(0x8140, 2), Some(633));
        assert_eq!(cmap.lookup_cid_code(0x817E, 2), Some(633 + 62));
    }

    #[test]
    fn cid_char_lookup() {
        let cmap = parse_with_preamble(
            br#"
3 begincidchar
<03> 1
<04> 2
<20> 50
endcidchar
"#,
        );

        assert_eq!(cmap.lookup_cid_code(0x03, 1), Some(1));
        assert_eq!(cmap.lookup_cid_code(0x04, 1), Some(2));
        assert_eq!(cmap.lookup_cid_code(0x20, 1), Some(50));
    }

    #[test]
    fn lookup_miss() {
        let cmap = parse_with_preamble(
            br#"
1 begincidrange
<0100> <01FF> 0
endcidrange
"#,
        );

        assert_eq!(cmap.lookup_cid_code(0x00FF, 2), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x0200, 2), Some(0));
        assert_eq!(cmap.lookup_cid_code(0xFFFF, 2), Some(0));
    }

    #[test]
    fn multiple_sections() {
        let cmap = parse_with_preamble(
            br#"
2 begincidrange
<0000> <00FF> 0
<0100> <01FF> 256
endcidrange
1 begincidchar
<0200> 600
endcidchar
1 begincidrange
<8140> <817E> 633
endcidrange
"#,
        );

        assert_eq!(cmap.lookup_cid_code(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x0100, 2), Some(256));
        assert_eq!(cmap.lookup_cid_code(0x0200, 2), Some(600));
        assert_eq!(cmap.lookup_cid_code(0x8140, 2), Some(633));
    }

    #[test]
    fn single_byte_codes() {
        let cmap = parse_with_preamble(
            br#"
2 begincidrange
<00> <7F> 0
<80> <FF> 200
endcidrange
"#,
        );

        assert_eq!(cmap.lookup_cid_code(0x00, 1), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x41, 1), Some(0x41));
        assert_eq!(cmap.lookup_cid_code(0x80, 1), Some(200));
        assert_eq!(cmap.lookup_cid_code(0xFF, 1), Some(200 + 127));
    }

    #[test]
    fn usecmap_chaining() {
        let base_data = br#"
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 0 def
end def
/CMapName /Base def
/WMode 0 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 begincidrange
<0000> <00FF> 0
endcidrange
"#;

        let child_data = br#"
/Base usecmap
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 0 def
end def
/CMapName /Child def
/WMode 0 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 begincidrange
<0100> <01FF> 256
endcidrange
"#;

        let cmap = CMap::parse(child_data, |name| {
            if name == b"Base" {
                Some(base_data.as_slice())
            } else {
                None
            }
        })
        .unwrap();

        assert_eq!(cmap.lookup_cid_code(0x0100, 2), Some(256));
        assert_eq!(cmap.lookup_cid_code(0x01FF, 2), Some(511));
        assert_eq!(cmap.lookup_cid_code(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x00FF, 2), Some(0xFF));

        assert_eq!(cmap.lookup_cid_code(0x0200, 2), Some(0));
    }

    #[test]
    fn usecmap_partial_override() {
        let base_data = br#"
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 0 def
end def
/CMapName /Base def
/WMode 0 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 begincidrange
<0000> <00FF> 0
endcidrange
"#;

        let child_data = br#"
/Base usecmap
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 0 def
end def
/CMapName /Child def
/WMode 0 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 begincidrange
<0040> <007F> 500
endcidrange
"#;

        let cmap = CMap::parse(child_data, |name| {
            if name == b"Base" {
                Some(base_data.as_slice())
            } else {
                None
            }
        })
        .unwrap();

        assert_eq!(cmap.lookup_cid_code(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x003F, 2), Some(0x3F));
        assert_eq!(cmap.lookup_cid_code(0x0040, 2), Some(500));
        assert_eq!(cmap.lookup_cid_code(0x007F, 2), Some(563));
        assert_eq!(cmap.lookup_cid_code(0x0080, 2), Some(0x80));
        assert_eq!(cmap.lookup_cid_code(0x00FF, 2), Some(0xFF));
    }

    #[test]
    fn notdef_char_lookup() {
        let cmap = parse_with_preamble(
            br#"
2 beginnotdefchar
<03> 10
<20> 20
endnotdefchar
"#,
        );

        assert_eq!(cmap.lookup_cid_code(0x03, 1), Some(10));
        assert_eq!(cmap.lookup_cid_code(0x20, 1), Some(20));
        assert_eq!(cmap.lookup_cid_code(0x04, 1), Some(0));
    }

    #[test]
    fn notdef_range_lookup() {
        let cmap = parse_with_preamble(
            br#"
1 beginnotdefrange
<0000> <001F> 100
endnotdefrange
"#,
        );

        assert_eq!(cmap.lookup_cid_code(0x0000, 2), Some(100));
        assert_eq!(cmap.lookup_cid_code(0x0001, 2), Some(100));
        assert_eq!(cmap.lookup_cid_code(0x001F, 2), Some(100));
        assert_eq!(cmap.lookup_cid_code(0x0020, 2), Some(0));
    }

    #[test]
    fn bfchar_lookup() {
        let cmap = parse_with_preamble(
            br#"
2 beginbfchar
<0041> <0048>
<0042> <0065>
endbfchar
"#,
        );

        assert_eq!(
            cmap.lookup_unicode_code(0x0041),
            Some(UnicodeString::Char('H'))
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x0042),
            Some(UnicodeString::Char('e'))
        );
        assert_eq!(cmap.lookup_unicode_code(0x0043), None);
    }

    #[test]
    fn bfchar_ligature() {
        let cmap = parse_with_preamble(
            br#"
1 beginbfchar
<005F> <00660066>
endbfchar
"#,
        );

        assert_eq!(
            cmap.lookup_unicode_code(0x005F),
            Some(UnicodeString::String(String::from("ff")))
        );
    }

    #[test]
    fn bfchar_surrogate_pair() {
        let cmap = parse_with_preamble(
            br#"
1 beginbfchar
<3A51> <D840DC3E>
endbfchar
"#,
        );

        assert_eq!(
            cmap.lookup_unicode_code(0x3A51),
            Some(UnicodeString::Char('\u{2003E}'))
        );
    }

    #[test]
    fn bfrange_incrementing() {
        let cmap = parse_with_preamble(
            br#"
1 beginbfrange
<0000> <0004> <0041>
endbfrange
"#,
        );

        assert_eq!(
            cmap.lookup_unicode_code(0x0000),
            Some(UnicodeString::Char('A'))
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x0001),
            Some(UnicodeString::Char('B'))
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x0004),
            Some(UnicodeString::Char('E'))
        );
        assert_eq!(cmap.lookup_unicode_code(0x0005), None);
    }

    #[test]
    fn bfrange_array() {
        let cmap = parse_with_preamble(
            br#"
1 beginbfrange
<005F> <0061> [<00660066> <00660069> <0066006C>]
endbfrange
"#,
        );

        // ff, fi, fl ligatures
        assert_eq!(
            cmap.lookup_unicode_code(0x005F),
            Some(UnicodeString::String(String::from("ff")))
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x0060),
            Some(UnicodeString::String(String::from("fi")))
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x0061),
            Some(UnicodeString::String(String::from("fl")))
        );
    }

    #[test]
    fn unicode_lookup_miss() {
        let cmap = parse_with_preamble(
            br#"
1 beginbfchar
<0041> <0048>
endbfchar
"#,
        );

        assert_eq!(cmap.lookup_unicode_code(0x0000), None);
        assert_eq!(cmap.lookup_unicode_code(0x0042), None);
    }

    #[test]
    fn identity_h() {
        let cmap = CMap::identity_h();
        assert_eq!(cmap.metadata().name, b"Identity-H");
        assert_eq!(cmap.metadata().writing_mode, WritingMode::Horizontal);

        assert_eq!(cmap.lookup_cid_code(0x0041, 2), Some(0x0041));
        assert_eq!(cmap.lookup_cid_code(0x1234, 2), Some(0x1234));
        assert_eq!(cmap.lookup_cid_code(0xFFFF, 2), Some(0xFFFF));

        assert_eq!(cmap.lookup_cid_code(0x0041, 1), None);
        assert_eq!(cmap.lookup_cid_code(0x0041, 3), None);
    }

    #[test]
    fn identity_v() {
        let cmap = CMap::identity_v();
        assert_eq!(cmap.metadata().name, b"Identity-V");
        assert_eq!(cmap.metadata().writing_mode, WritingMode::Vertical);

        assert_eq!(cmap.lookup_cid_code(0x0041, 2), Some(0x0041));
        assert_eq!(cmap.lookup_cid_code(0xFFFF, 2), Some(0xFFFF));
    }

    #[test]
    fn codespace_range_mixed() {
        let data = br#"
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 0 def
end def
/CMapName /Test def
/WMode 0 def
2 begincodespacerange
<00> <80>
<8140> <9FFC>
endcodespacerange
1 begincidrange
<00> <80> 0
endcidrange
1 begincidrange
<8140> <9FFC> 200
endcidrange
"#;
        let cmap = CMap::parse(data.as_slice(), |_| None).unwrap();

        assert_eq!(cmap.lookup_cid_code(0x41, 1), Some(0x41));
        assert_eq!(cmap.lookup_cid_code(0x00, 1), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x80, 1), Some(0x80));
        assert_eq!(cmap.lookup_cid_code(0x81, 1), None);

        assert_eq!(cmap.lookup_cid_code(0x8140, 2), Some(200));
        assert_eq!(cmap.lookup_cid_code(0x9FFC, 2), Some(200 + 0x9FFC - 0x8140));
        assert_eq!(cmap.lookup_cid_code(0x8100, 2), None);

        assert_eq!(cmap.lookup_cid_code(0x41, 2), None);
    }

    #[test]
    fn codespace_range_4_byte() {
        let cmap = parse_with_preamble(
            br#"
1 begincodespacerange
<8EA1A1A1> <8EA1FEFE>
endcodespacerange
"#,
        );

        assert_eq!(cmap.lookup_cid_code(0x8EA1A1A1, 4), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x8EA1FEFE, 4), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x8EA1A1A0, 4), None);
        assert_eq!(cmap.lookup_cid_code(0x8EA1A1A1, 3), None);
    }
}
