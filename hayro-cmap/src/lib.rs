/*!
A CMap parser.

This crate provides a parser for CMap files, which are used in PDF to map
character codes to CID (Character Identifier) values.

## Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![no_std]
#![forbid(unsafe_code)]
#![allow(missing_docs)]

extern crate alloc;

mod parse;
mod scanner_ext;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// The name of a CMap.
pub type CMapName<'a> = &'a [u8];

/// A CID (Character Identifier).
pub type Cid = u32;

/// Don't allow more than 16 `usecmap` references.
const MAX_NESTING_DEPTH: u32 = 16;

/// A parsed CMap.
#[derive(Debug, Clone)]
pub struct CMap {
    metadata: Metadata,
    ranges: Vec<CidRange>,
    notdef_ranges: Vec<CidRange>,
    bf_entries: Vec<BfRange>,
    base: Option<Box<CMap>>,
}

impl CMap {
    /// Parse a CMap from raw bytes.
    ///
    /// The `get_cmap` callback is used to resolve CMaps that are referenced
    /// via `usecmap`.
    pub fn parse<'a>(
        data: &[u8],
        get_cmap: impl Fn(CMapName<'_>) -> Option<&'a [u8]> + Clone + 'a,
    ) -> Option<Self> {
        parse::parse(data, get_cmap, 0)
    }

    pub(crate) fn new(
        metadata: Metadata,
        ranges: Vec<CidRange>,
        notdef_ranges: Vec<CidRange>,
        bf_entries: Vec<BfRange>,
        base: Option<Box<CMap>>,
    ) -> Self {
        Self {
            metadata,
            ranges,
            notdef_ranges,
            bf_entries,
            base,
        }
    }

    /// Return the metadata of this CMap.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Look up a character code and return the corresponding CID.
    pub fn lookup_cid(&self, code: u32) -> Option<Cid> {
        if let Some(range) = find_range(&self.ranges, code) {
            let offset = code.checked_sub(range.start)?;
            return Some(range.cid_start + offset);
        } else if let Some(range) = find_range(&self.notdef_ranges, code) {
            return Some(range.cid_start);
        }

        self.base.as_ref()?.lookup_cid(code)
    }

    /// Look up a character code and return the corresponding Unicode value.
    pub fn lookup_unicode(&self, code: u32) -> Option<UnicodeString> {
        if let Some(entry) = find_range_in_bf(&self.bf_entries, code) {
            let offset = u16::try_from(code - entry.start).ok()?;
            let mut units = entry.dst_base.clone();
            let last = units.last_mut()?;
            *last = last.checked_add(offset)?;
            let s = String::from_utf16(&units).ok()?;
            let mut chars = s.chars();
            let first = chars.next()?;
            return if chars.next().is_none() {
                Some(UnicodeString::Char(first))
            } else {
                Some(UnicodeString::String(s))
            };
        }

        self.base.as_ref()?.lookup_unicode(code)
    }
}

fn find_range(ranges: &[CidRange], code: u32) -> Option<&CidRange> {
    let idx = ranges
        .binary_search_by(|range| {
            if code < range.start {
                core::cmp::Ordering::Greater
            } else if code > range.end {
                core::cmp::Ordering::Less
            } else {
                core::cmp::Ordering::Equal
            }
        })
        .ok()?;

    Some(&ranges[idx])
}

fn find_range_in_bf(entries: &[BfRange], code: u32) -> Option<&BfRange> {
    let idx = entries
        .binary_search_by(|entry| {
            if code < entry.start {
                core::cmp::Ordering::Greater
            } else if code > entry.end {
                core::cmp::Ordering::Less
            } else {
                core::cmp::Ordering::Equal
            }
        })
        .ok()?;

    Some(&entries[idx])
}

/// A range of character codes mapped to CIDs.
#[derive(Debug, Clone)]
pub struct CidRange {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) cid_start: Cid,
}

/// A character code to Unicode mapping (potentially a range).
#[derive(Debug, Clone)]
pub(crate) struct BfRange {
    pub(crate) start: u32,
    pub(crate) end: u32,
    /// UTF-16 code units. For ranges, the last unit is incremented by the offset.
    pub(crate) dst_base: Vec<u16>,
}

/// A Unicode value decoded from a ToUnicode CMap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnicodeString {
    /// A single Unicode character.
    Char(char),
    /// A string consisting of multiple Unicode characters, stored as a UTF-8 string.
    String(String),
}

/// Metadata extracted from a CMap file.
#[derive(Debug, Clone)]
pub struct Metadata {
    pub registry: String,
    pub ordering: String,
    pub supplement: i32,
    pub name: String,
    pub writing_mode: WritingMode,
}

/// The writing mode of a CMap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    #[default]
    Horizontal,
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
        assert_eq!(cmap.metadata().registry, "Adobe");
        assert_eq!(cmap.metadata().ordering, "Japan1");
        assert_eq!(cmap.metadata().supplement, 6);
        assert_eq!(cmap.metadata().name, "Adobe-Japan1-H");
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
        assert_eq!(cmap.metadata().name, "Adobe-Japan1-V");
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

        // First range: <0000>-<00FF> -> CID 0-255
        assert_eq!(cmap.lookup_cid(0x0000), Some(0));
        assert_eq!(cmap.lookup_cid(0x0042), Some(0x42));
        assert_eq!(cmap.lookup_cid(0x00FF), Some(0xFF));

        // Second range: <0100>-<01FF> -> CID 256-511
        assert_eq!(cmap.lookup_cid(0x0100), Some(256));
        assert_eq!(cmap.lookup_cid(0x01FF), Some(511));

        // Third range: <8140>-<817E> -> CID 633-695
        assert_eq!(cmap.lookup_cid(0x8140), Some(633));
        assert_eq!(cmap.lookup_cid(0x817E), Some(633 + 62));
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

        assert_eq!(cmap.lookup_cid(0x03), Some(1));
        assert_eq!(cmap.lookup_cid(0x04), Some(2));
        assert_eq!(cmap.lookup_cid(0x20), Some(50));
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

        assert_eq!(cmap.lookup_cid(0x00FF), None);
        assert_eq!(cmap.lookup_cid(0x0200), None);
        assert_eq!(cmap.lookup_cid(0xFFFF), None);
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

        assert_eq!(cmap.lookup_cid(0x0000), Some(0));
        assert_eq!(cmap.lookup_cid(0x0100), Some(256));
        assert_eq!(cmap.lookup_cid(0x0200), Some(600));
        assert_eq!(cmap.lookup_cid(0x8140), Some(633));
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

        assert_eq!(cmap.lookup_cid(0x00), Some(0));
        assert_eq!(cmap.lookup_cid(0x41), Some(0x41));
        assert_eq!(cmap.lookup_cid(0x80), Some(200));
        assert_eq!(cmap.lookup_cid(0xFF), Some(200 + 127));
    }

    #[test]
    fn missing_metadata_fails() {
        assert!(CMap::parse(b"", |_| None).is_none());
        assert!(CMap::parse(b"/CMapName /X def", |_| None).is_none());
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

        assert_eq!(cmap.lookup_cid(0x0100), Some(256));
        assert_eq!(cmap.lookup_cid(0x01FF), Some(511));
        assert_eq!(cmap.lookup_cid(0x0000), Some(0));
        assert_eq!(cmap.lookup_cid(0x00FF), Some(0xFF));

        assert_eq!(cmap.lookup_cid(0x0200), None);
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

        assert_eq!(cmap.lookup_cid(0x0000), Some(0));
        assert_eq!(cmap.lookup_cid(0x003F), Some(0x3F));
        assert_eq!(cmap.lookup_cid(0x0040), Some(500));
        assert_eq!(cmap.lookup_cid(0x007F), Some(563));
        assert_eq!(cmap.lookup_cid(0x0080), Some(0x80));
        assert_eq!(cmap.lookup_cid(0x00FF), Some(0xFF));
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

        assert_eq!(cmap.lookup_cid(0x03), Some(10));
        assert_eq!(cmap.lookup_cid(0x20), Some(20));
        assert_eq!(cmap.lookup_cid(0x04), None);
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

        assert_eq!(cmap.lookup_cid(0x0000), Some(100));
        assert_eq!(cmap.lookup_cid(0x0001), Some(100));
        assert_eq!(cmap.lookup_cid(0x001F), Some(100));
        assert_eq!(cmap.lookup_cid(0x0020), None);
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

        assert_eq!(cmap.lookup_unicode(0x0041), Some(UnicodeString::Char('H')));
        assert_eq!(cmap.lookup_unicode(0x0042), Some(UnicodeString::Char('e')));
        assert_eq!(cmap.lookup_unicode(0x0043), None);
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
            cmap.lookup_unicode(0x005F),
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
            cmap.lookup_unicode(0x3A51),
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

        assert_eq!(cmap.lookup_unicode(0x0000), Some(UnicodeString::Char('A')));
        assert_eq!(cmap.lookup_unicode(0x0001), Some(UnicodeString::Char('B')));
        assert_eq!(cmap.lookup_unicode(0x0004), Some(UnicodeString::Char('E')));
        assert_eq!(cmap.lookup_unicode(0x0005), None);
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
            cmap.lookup_unicode(0x005F),
            Some(UnicodeString::String(String::from("ff")))
        );
        assert_eq!(
            cmap.lookup_unicode(0x0060),
            Some(UnicodeString::String(String::from("fi")))
        );
        assert_eq!(
            cmap.lookup_unicode(0x0061),
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

        assert_eq!(cmap.lookup_unicode(0x0000), None);
        assert_eq!(cmap.lookup_unicode(0x0042), None);
    }
}
