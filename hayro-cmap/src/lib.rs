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

mod ext;
mod parse;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// The name of a CMap.
pub type CMapName<'a> = &'a [u8];

/// Don't allow more than 16 `usecmap` references.
const MAX_NESTING_DEPTH: u32 = 16;

/// A parsed CMap.
#[derive(Debug, Clone)]
pub struct CMap {
    metadata: Metadata,
    ranges: Vec<CidRange>,
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
        base: Option<Box<CMap>>,
    ) -> Self {
        Self {
            metadata,
            ranges,
            base,
        }
    }

    /// Return the metadata of this CMap.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Look up a character code and return the corresponding CID.
    pub fn lookup(&self, code: &CharacterCode) -> Option<u32> {
        let result = self.ranges.binary_search_by(|range| {
            if *code < range.start {
                Ordering::Greater
            } else if *code > range.end {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        });

        if let Ok(idx) = result {
            let range = &self.ranges[idx];
            let offset = code.offset_from(&range.start)?;
            
            return Some(range.cid_start + offset);
        }

        self.base.as_ref()?.lookup(code)
    }
}

/// A range of character codes mapped to CIDs.
#[derive(Debug, Clone)]
pub struct CidRange {
    pub(crate) start: CharacterCode,
    pub(crate) end: CharacterCode,
    pub(crate) cid_start: u32,
}

/// A character code in a CMap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterCode {
    /// A character code that fits in 4 bytes or fewer.
    Single(u32),
    /// A character code longer than 4 bytes.
    Multi(Vec<u8>),
}

impl CharacterCode {
    /// Create a `CharacterCode` from decoded bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        if bytes.len() <= 4 {
            let mut val = 0u32;
            for &b in bytes {
                val = (val << 8) | u32::from(b);
            }
            Self::Single(val)
        } else {
            Self::Multi(bytes.to_vec())
        }
    }

    fn offset_from(&self, start: &Self) -> Option<u32> {
        match (self, start) {
            (Self::Single(c), Self::Single(s)) => c.checked_sub(*s),
            (Self::Multi(c), Self::Multi(s)) if c.len() == s.len() => {
                let c_val: u64 = c.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
                let s_val: u64 = s.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
                u32::try_from(c_val.checked_sub(s_val)?).ok()
            }
            _ => None,
        }
    }
}

impl PartialOrd for CharacterCode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CharacterCode {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Single(a), Self::Single(b)) => a.cmp(b),
            (Self::Multi(a), Self::Multi(b)) => a.cmp(b),
            (Self::Single(_), Self::Multi(_)) => Ordering::Less,
            (Self::Multi(_), Self::Single(_)) => Ordering::Greater,
        }
    }
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
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0000)), Some(0));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0042)), Some(0x42));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x00FF)), Some(0xFF));

        // Second range: <0100>-<01FF> -> CID 256-511
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0100)), Some(256));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x01FF)), Some(511));

        // Third range: <8140>-<817E> -> CID 633-695
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x8140)), Some(633));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x817E)), Some(633 + 62));
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

        assert_eq!(cmap.lookup(&CharacterCode::Single(0x03)), Some(1));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x04)), Some(2));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x20)), Some(50));
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

        assert_eq!(cmap.lookup(&CharacterCode::Single(0x00FF)), None);
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0200)), None);
        assert_eq!(cmap.lookup(&CharacterCode::Single(0xFFFF)), None);
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

        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0000)), Some(0));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0100)), Some(256));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0200)), Some(600));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x8140)), Some(633));
    }

    #[test]
    fn char_code_from_bytes() {
        assert_eq!(
            CharacterCode::from_bytes(&[0x03]),
            CharacterCode::Single(0x03)
        );
        assert_eq!(
            CharacterCode::from_bytes(&[0x00, 0x41]),
            CharacterCode::Single(0x0041)
        );
        assert_eq!(
            CharacterCode::from_bytes(&[0x81, 0x40]),
            CharacterCode::Single(0x8140)
        );
        assert_eq!(
            CharacterCode::from_bytes(&[0x01, 0x02, 0x03, 0x04, 0x05]),
            CharacterCode::Multi(alloc::vec![0x01, 0x02, 0x03, 0x04, 0x05])
        );
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

        assert_eq!(cmap.lookup(&CharacterCode::Single(0x00)), Some(0));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x41)), Some(0x41));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x80)), Some(200));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0xFF)), Some(200 + 127));
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

        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0100)), Some(256));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x01FF)), Some(511));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0000)), Some(0));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x00FF)), Some(0xFF));

        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0200)), None);
    }

    #[test]
    fn usecmap_child_overrides_base() {
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
<0000> <00FF> 100
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

        // Child overrides base for the same range
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0000)), Some(100));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x00FF)), Some(100 + 0xFF));
    }
}
