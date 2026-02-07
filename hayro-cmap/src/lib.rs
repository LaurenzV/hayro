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

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// A parsed CMap.
#[derive(Debug, Clone)]
pub struct CMap {
    metadata: Metadata,
    ranges: Vec<CidRange>,
}

impl CMap {
    /// Parse a CMap from raw bytes.
    pub fn parse(data: &[u8]) -> Option<Self> {
        parse::parse(data)
    }

    pub(crate) fn new(metadata: Metadata, ranges: Vec<CidRange>) -> Self {
        Self { metadata, ranges }
    }

    /// Return the metadata of this CMap.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Look up a character code and return the corresponding CID.
    pub fn lookup(&self, code: &CharacterCode) -> Option<u32> {
        let idx = self
            .ranges
            .binary_search_by(|range| {
                if *code < range.start {
                    Ordering::Greater
                } else if *code > range.end {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .ok()?;

        let range = &self.ranges[idx];
        let offset = code.offset_from(&range.start)?;
        Some(range.cid_start + offset)
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
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub registry: Option<String>,
    pub ordering: Option<String>,
    pub supplement: Option<i32>,
    pub name: Option<String>,
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
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
endcmap"#;

        let cmap = CMap::parse(data).unwrap();
        assert_eq!(cmap.metadata().registry.as_deref(), Some("Adobe"));
        assert_eq!(cmap.metadata().ordering.as_deref(), Some("Japan1"));
        assert_eq!(cmap.metadata().supplement, Some(6));
        assert_eq!(cmap.metadata().name.as_deref(), Some("Adobe-Japan1-H"));
        assert_eq!(cmap.metadata().writing_mode, WritingMode::Horizontal);
    }

    #[test]
    fn vertical_writing_mode() {
        let data = br#"
begincmap
/CIDSystemInfo 3 dict dup begin
  /Registry (Adobe) def
  /Ordering (Japan1) def
  /Supplement 6 def
end def
/CMapName /Adobe-Japan1-V def
/WMode 1 def
endcmap
"#;

        let cmap = CMap::parse(data).unwrap();
        assert_eq!(cmap.metadata().writing_mode, WritingMode::Vertical);
        assert_eq!(cmap.metadata().name.as_deref(), Some("Adobe-Japan1-V"));
    }

    #[test]
    fn cid_range_lookup() {
        let data = br#"
begincmap
3 begincidrange
<0000> <00FF> 0
<0100> <01FF> 256
<8140> <817E> 633
endcidrange
endcmap
"#;

        let cmap = CMap::parse(data).unwrap();

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
        let data = br#"
begincmap
3 begincidchar
<03> 1
<04> 2
<20> 50
endcidchar
endcmap
"#;

        let cmap = CMap::parse(data).unwrap();
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x03)), Some(1));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x04)), Some(2));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x20)), Some(50));
    }

    #[test]
    fn lookup_miss() {
        let data = br#"
begincmap
1 begincidrange
<0100> <01FF> 0
endcidrange
endcmap
"#;

        let cmap = CMap::parse(data).unwrap();
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x00FF)), None);
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x0200)), None);
        assert_eq!(cmap.lookup(&CharacterCode::Single(0xFFFF)), None);
    }

    #[test]
    fn multiple_sections() {
        let data = br#"
begincmap
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
endcmap
"#;

        let cmap = CMap::parse(data).unwrap();
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
        let data = br#"
begincmap
2 begincidrange
<00> <7F> 0
<80> <FF> 200
endcidrange
endcmap
"#;

        let cmap = CMap::parse(data).unwrap();
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x00)), Some(0));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x41)), Some(0x41));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0x80)), Some(200));
        assert_eq!(cmap.lookup(&CharacterCode::Single(0xFF)), Some(200 + 127));
    }

    #[test]
    fn empty_cmap() {
        let cmap = CMap::parse(b"").unwrap();
        assert!(cmap.metadata().registry.is_none());
        assert!(cmap.metadata().name.is_none());
        assert_eq!(cmap.lookup(&CharacterCode::Single(0)), None);
    }
}
