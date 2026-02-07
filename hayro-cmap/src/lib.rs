/*!
A parser for CMap files, as they are found in PDFs.

This crate provides a parser for CMap files and allows you to
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
use hayro_postscript::Scanner;

/// The name of a CMap.
pub type CMapName<'a> = &'a [u8];
/// A CID (Character Identifier).
pub type Cid = u32;

/// Let's limit the number of nested `usecmap` references to 16.
const MAX_NESTING_DEPTH: u32 = 16;

/// A parsed CMap.
#[derive(Debug, Clone)]
pub struct CMap {
    metadata: Metadata,
    codespace_ranges: Vec<CodespaceRange>,
    cid_ranges: Vec<CidRange>,
    notdef_ranges: Vec<CidRange>,
    bf_entries: Vec<BfRange>,
    base: Option<Box<CMap>>,
}

impl CMap {
    /// Parse a CMap from raw bytes.
    ///
    /// The `get_cmap` callback is used to recursively resolve CMaps that
    /// are referenced via `usecmap`.
    pub fn parse<'a>(
        data: &[u8],
        get_cmap: impl Fn(CMapName<'_>) -> Option<&'a [u8]> + Clone + 'a,
    ) -> Option<Self> {
        parse::parse(data, get_cmap, 0)
    }

    /// Create an Identity-H CMap.
    pub fn identity_h() -> Self {
        Self::identity(WritingMode::Horizontal, "Identity-H")
    }

    /// Create an Identity-V CMap.
    pub fn identity_v() -> Self {
        Self::identity(WritingMode::Vertical, "Identity-V")
    }

    fn identity(writing_mode: WritingMode, name: &str) -> Self {
        Self {
            metadata: Metadata {
                registry: String::from("Adobe"),
                ordering: String::from("Identity"),
                supplement: 0,
                name: String::from(name),
                writing_mode,
            },
            codespace_ranges: vec![CodespaceRange {
                n_bytes: 2,
                low: 0,
                high: 0xFFFF,
            }],
            cid_ranges: vec![CidRange {
                start: 0,
                end: 0xFFFF,
                cid_start: 0,
            }],
            notdef_ranges: Vec::new(),
            bf_entries: Vec::new(),
            base: None,
        }
    }

    /// Return the metadata of this CMap.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Look up the CID of a character code.
    ///
    /// Returns `None` if the code is not within any codespace range for the
    /// given byte length.
    pub fn lookup_cid(&self, code: u32, byte_len: u8) -> Option<Cid> {
        let in_codespace = self
            .codespace_ranges
            .iter()
            .any(|r| r.n_bytes == byte_len && code >= r.low && code <= r.high);

        if !in_codespace {
            return None;
        }

        if let Some(range) = find_range(&self.cid_ranges, code) {
            let offset = code.checked_sub(range.start)?;
            
            return Some(range.cid_start + offset);
        } else if let Some(range) = find_range(&self.notdef_ranges, code) {
            // For `.notdef` ranges, all codes map to the same `.notdef` CID, so
            // no adding of the offset here.
            return Some(range.cid_start);
        }

        // If character code is in code space range but has no active mapping, so
        // assume `.notdef`.
        Some(
            self.base
                .as_ref()
                .and_then(|b| b.lookup_cid(code, byte_len))
                .unwrap_or(0),
        )
    }

    /// Look up the Unicode string of the given character code.
    /// 
    /// Returns `None` if no mapping is available.
    pub fn lookup_unicode(&self, code: u32) -> Option<UnicodeString> {
        if let Some(entry) = find_range_in_bf(&self.bf_entries, code) {
            let offset = u16::try_from(code - entry.start).ok()?;

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

            if offset == 0 {
                return Some(decode_utf16(&entry.dst_base)?);
            }  

            let mut units = entry.dst_base.clone();
            *units.last_mut()? = units.last()?.checked_add(offset)?;
            return Some(decode_utf16(&units)?);
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

/// A codespace range defining valid character code byte sequences.
#[derive(Debug, Clone)]
pub(crate) struct CodespaceRange {
    /// Number of bytes in this codespace range (1–4).
    pub(crate) n_bytes: u8,
    pub(crate) low: u32,
    pub(crate) high: u32,
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
    /// The registry name (e.g. "Adobe").
    pub registry: String,
    /// The ordering name (e.g. "Japan1").
    pub ordering: String,
    /// The supplement number.
    pub supplement: i32,
    /// The CMap name.
    pub name: String,
    /// The writing mode.
    pub writing_mode: WritingMode,
}

/// The writing mode of a CMap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    /// Horizontal writing mode.
    #[default]
    Horizontal,
    /// Vertical writing mode.
    Vertical,
}

pub(crate) trait ScannerExt {
    fn read_string(&mut self, buf: &mut Vec<u8>) -> Option<String>;
    fn read_integer(&mut self) -> Option<i32>;
    fn read_u32_code(&mut self, buf: &mut Vec<u8>) -> Option<u32>;
}

impl ScannerExt for Scanner<'_> {
    fn read_string(&mut self, buf: &mut Vec<u8>) -> Option<String> {
        let s = self.parse_string().ok()?;
        s.decode_into(buf).ok()?;
        String::from_utf8(buf.to_vec()).ok()
    }

    fn read_integer(&mut self) -> Option<i32> {
        let n = self.parse_number().ok()?;
        Some(n.as_i32())
    }

    fn read_u32_code(&mut self, buf: &mut Vec<u8>) -> Option<u32> {
        let s = self.parse_string().ok()?;
        s.decode_into(buf).ok()?;
        if buf.len() > 4 {
            return None;
        }
        let mut val = 0u32;
        for &b in buf.iter() {
            val = (val << 8) | u32::from(b);
        }
        Some(val)
    }
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

        assert_eq!(cmap.lookup_cid(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid(0x0042, 2), Some(0x42));
        assert_eq!(cmap.lookup_cid(0x00FF, 2), Some(0xFF));

        assert_eq!(cmap.lookup_cid(0x0100, 2), Some(256));
        assert_eq!(cmap.lookup_cid(0x01FF, 2), Some(511));

        assert_eq!(cmap.lookup_cid(0x8140, 2), Some(633));
        assert_eq!(cmap.lookup_cid(0x817E, 2), Some(633 + 62));
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

        assert_eq!(cmap.lookup_cid(0x03, 1), Some(1));
        assert_eq!(cmap.lookup_cid(0x04, 1), Some(2));
        assert_eq!(cmap.lookup_cid(0x20, 1), Some(50));
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

        assert_eq!(cmap.lookup_cid(0x00FF, 2), Some(0));
        assert_eq!(cmap.lookup_cid(0x0200, 2), Some(0));
        assert_eq!(cmap.lookup_cid(0xFFFF, 2), Some(0));
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

        assert_eq!(cmap.lookup_cid(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid(0x0100, 2), Some(256));
        assert_eq!(cmap.lookup_cid(0x0200, 2), Some(600));
        assert_eq!(cmap.lookup_cid(0x8140, 2), Some(633));
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

        assert_eq!(cmap.lookup_cid(0x00, 1), Some(0));
        assert_eq!(cmap.lookup_cid(0x41, 1), Some(0x41));
        assert_eq!(cmap.lookup_cid(0x80, 1), Some(200));
        assert_eq!(cmap.lookup_cid(0xFF, 1), Some(200 + 127));
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

        assert_eq!(cmap.lookup_cid(0x0100, 2), Some(256));
        assert_eq!(cmap.lookup_cid(0x01FF, 2), Some(511));
        assert_eq!(cmap.lookup_cid(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid(0x00FF, 2), Some(0xFF));

        assert_eq!(cmap.lookup_cid(0x0200, 2), Some(0));
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

        assert_eq!(cmap.lookup_cid(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid(0x003F, 2), Some(0x3F));
        assert_eq!(cmap.lookup_cid(0x0040, 2), Some(500));
        assert_eq!(cmap.lookup_cid(0x007F, 2), Some(563));
        assert_eq!(cmap.lookup_cid(0x0080, 2), Some(0x80));
        assert_eq!(cmap.lookup_cid(0x00FF, 2), Some(0xFF));
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

        assert_eq!(cmap.lookup_cid(0x03, 1), Some(10));
        assert_eq!(cmap.lookup_cid(0x20, 1), Some(20));
        assert_eq!(cmap.lookup_cid(0x04, 1), Some(0));
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

        assert_eq!(cmap.lookup_cid(0x0000, 2), Some(100));
        assert_eq!(cmap.lookup_cid(0x0001, 2), Some(100));
        assert_eq!(cmap.lookup_cid(0x001F, 2), Some(100));
        assert_eq!(cmap.lookup_cid(0x0020, 2), Some(0));
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

    #[test]
    fn identity_h() {
        let cmap = CMap::identity_h();
        assert_eq!(cmap.metadata().name, "Identity-H");
        assert_eq!(cmap.metadata().writing_mode, WritingMode::Horizontal);

        assert_eq!(cmap.lookup_cid(0x0041, 2), Some(0x0041));
        assert_eq!(cmap.lookup_cid(0x1234, 2), Some(0x1234));
        assert_eq!(cmap.lookup_cid(0xFFFF, 2), Some(0xFFFF));

        assert_eq!(cmap.lookup_cid(0x0041, 1), None);
        assert_eq!(cmap.lookup_cid(0x0041, 3), None);
    }

    #[test]
    fn identity_v() {
        let cmap = CMap::identity_v();
        assert_eq!(cmap.metadata().name, "Identity-V");
        assert_eq!(cmap.metadata().writing_mode, WritingMode::Vertical);

        assert_eq!(cmap.lookup_cid(0x0041, 2), Some(0x0041));
        assert_eq!(cmap.lookup_cid(0xFFFF, 2), Some(0xFFFF));
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

        assert_eq!(cmap.lookup_cid(0x41, 1), Some(0x41));
        assert_eq!(cmap.lookup_cid(0x00, 1), Some(0));
        assert_eq!(cmap.lookup_cid(0x80, 1), Some(0x80));
        assert_eq!(cmap.lookup_cid(0x81, 1), None);

        assert_eq!(cmap.lookup_cid(0x8140, 2), Some(200));
        assert_eq!(cmap.lookup_cid(0x9FFC, 2), Some(200 + 0x9FFC - 0x8140));
        assert_eq!(cmap.lookup_cid(0x8100, 2), None);

        assert_eq!(cmap.lookup_cid(0x41, 2), None);
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

        assert_eq!(cmap.lookup_cid(0x8EA1A1A1, 4), Some(0));
        assert_eq!(cmap.lookup_cid(0x8EA1FEFE, 4), Some(0));
        assert_eq!(cmap.lookup_cid(0x8EA1A1A0, 4), None);
        assert_eq!(cmap.lookup_cid(0x8EA1A1A1, 3), None);
    }
}
