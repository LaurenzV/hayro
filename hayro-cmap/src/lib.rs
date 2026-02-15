/*!
A parser for cmap files, as they are found in PDFs.

This crate provides a parser for cmap files and allows you to
- Map character codes from text-showing operators to CID identifiers.
- Map CIDs to Unicode characters or strings.

## Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

#[cfg(feature = "embed-cmaps")]
mod bcmap;
mod parse;

#[cfg(feature = "embed-cmaps")]
pub use bcmap::load_embedded;

/// Look up an embedded binary cmap by name.
///
/// Returns `None` when the `embed-cmaps` feature is not enabled.
#[cfg(not(feature = "embed-cmaps"))]
pub fn load_embedded(_name: CMapName<'_>) -> Option<&'static [u8]> {
    None
}

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// A CID (Character Identifier).
pub type Cid = u32;

/// The name of the cmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CMapName<'a> {
    // ── Adobe-Japan1 (Japanese) ── Shift-JIS (RKSJ) encodings ──
    /// `83pv-RKSJ-H` — Adobe-Japan1, Mac Shift-JIS (`KanjiTalk` 6), horizontal.
    N83pvRksjH,
    /// `90ms-RKSJ-H` — Adobe-Japan1, Windows Shift-JIS (code page 932), horizontal.
    N90msRksjH,
    /// `90ms-RKSJ-V` — Adobe-Japan1, Windows Shift-JIS (code page 932), vertical.
    N90msRksjV,
    /// `90msp-RKSJ-H` — Adobe-Japan1, Windows Shift-JIS with proportional Roman, horizontal.
    N90mspRksjH,
    /// `90msp-RKSJ-V` — Adobe-Japan1, Windows Shift-JIS with proportional Roman, vertical.
    N90mspRksjV,
    /// `90pv-RKSJ-H` — Adobe-Japan1, Mac Shift-JIS (`KanjiTalk` 7), horizontal.
    N90pvRksjH,
    /// `Add-RKSJ-H` — Adobe-Japan1, Fujitsu FMR Shift-JIS, horizontal.
    AddRksjH,
    /// `Add-RKSJ-V` — Adobe-Japan1, Fujitsu FMR Shift-JIS, vertical.
    AddRksjV,

    // ── Adobe-CNS1 (Traditional Chinese) ── Big Five encodings ──
    /// `B5pc-H` — Adobe-CNS1, Mac Big Five (`ETen` extensions), horizontal.
    B5pcH,
    /// `B5pc-V` — Adobe-CNS1, Mac Big Five (`ETen` extensions), vertical.
    B5pcV,
    /// `CNS-EUC-H` — Adobe-CNS1, CNS 11643 EUC encoding, horizontal.
    CnsEucH,
    /// `CNS-EUC-V` — Adobe-CNS1, CNS 11643 EUC encoding, vertical.
    CnsEucV,
    /// `ETen-B5-H` — Adobe-CNS1, `ETen` Big Five extensions, horizontal.
    ETenB5H,
    /// `ETen-B5-V` — Adobe-CNS1, `ETen` Big Five extensions, vertical.
    ETenB5V,
    /// `ETenms-B5-H` — Adobe-CNS1, `ETen` Big Five with Microsoft symbol extensions, horizontal.
    ETenmsB5H,
    /// `ETenms-B5-V` — Adobe-CNS1, `ETen` Big Five with Microsoft symbol extensions, vertical.
    ETenmsB5V,

    // ── Adobe-Japan1 (Japanese) ── EUC and extended Shift-JIS encodings ──
    /// `EUC-H` — Adobe-Japan1, JIS X 0208 EUC-JP encoding, horizontal.
    EucH,
    /// `EUC-V` — Adobe-Japan1, JIS X 0208 EUC-JP encoding, vertical.
    EucV,
    /// `Ext-RKSJ-H` — Adobe-Japan1, Shift-JIS with NEC/IBM extensions, horizontal.
    ExtRksjH,
    /// `Ext-RKSJ-V` — Adobe-Japan1, Shift-JIS with NEC/IBM extensions, vertical.
    ExtRksjV,

    // ── Adobe-GB1 (Simplified Chinese) ──
    /// `GB-EUC-H` — Adobe-GB1, GB 2312-80 EUC encoding, horizontal.
    GbEucH,
    /// `GB-EUC-V` — Adobe-GB1, GB 2312-80 EUC encoding, vertical.
    GbEucV,
    /// `GBK-EUC-H` — Adobe-GB1, GBK encoding (Microsoft code page 936), horizontal.
    GbkEucH,
    /// `GBK-EUC-V` — Adobe-GB1, GBK encoding (Microsoft code page 936), vertical.
    GbkEucV,
    /// `GBK2K-H` — Adobe-GB1, GB 18030-2000 encoding, horizontal.
    Gbk2kH,
    /// `GBK2K-V` — Adobe-GB1, GB 18030-2000 encoding, vertical.
    Gbk2kV,
    /// `GBKp-EUC-H` — Adobe-GB1, GBK with proportional Roman, horizontal.
    GbkpEucH,
    /// `GBKp-EUC-V` — Adobe-GB1, GBK with proportional Roman, vertical.
    GbkpEucV,
    /// `GBpc-EUC-H` — Adobe-GB1, Mac GB 2312 (simplified) EUC encoding, horizontal.
    GbpcEucH,
    /// `GBpc-EUC-V` — Adobe-GB1, Mac GB 2312 (simplified) EUC encoding, vertical.
    GbpcEucV,

    // ── Adobe-Japan1 (Japanese) ── JIS encoding ──
    /// `H` — Adobe-Japan1, JIS X 0208 row-cell encoding, horizontal.
    H,

    // ── Adobe-CNS1 (Traditional Chinese) ── Hong Kong SCS ──
    /// `HKscs-B5-H` — Adobe-CNS1, Hong Kong SCS (Big Five with HKSCS extensions), horizontal.
    HKscsB5H,
    /// `HKscs-B5-V` — Adobe-CNS1, Hong Kong SCS (Big Five with HKSCS extensions), vertical.
    HKscsB5V,

    // ── Adobe-Identity ──
    /// `Identity-H` — Adobe-Identity, two-byte identity mapping, horizontal.
    /// Character codes map directly to CIDs (i.e. CID = character code).
    IdentityH,
    /// `Identity-V` — Adobe-Identity, two-byte identity mapping, vertical.
    /// Character codes map directly to CIDs (i.e. CID = character code).
    IdentityV,

    // ── Adobe-Korea1 (Korean) ──
    /// `KSC-EUC-H` — Adobe-Korea1, KS X 1001:1992 EUC-KR encoding, horizontal.
    KscEucH,
    /// `KSC-EUC-V` — Adobe-Korea1, KS X 1001:1992 EUC-KR encoding, vertical.
    KscEucV,
    /// `KSCms-UHC-H` — Adobe-Korea1, Microsoft UHC (Unified Hangul Code, code page 949), horizontal.
    KscmsUhcH,
    /// `KSCms-UHC-HW-H` — Adobe-Korea1, Microsoft UHC with half-width Roman, horizontal.
    KscmsUhcHwH,
    /// `KSCms-UHC-HW-V` — Adobe-Korea1, Microsoft UHC with half-width Roman, vertical.
    KscmsUhcHwV,
    /// `KSCms-UHC-V` — Adobe-Korea1, Microsoft UHC (Unified Hangul Code, code page 949), vertical.
    KscmsUhcV,
    /// `KSCpc-EUC-H` — Adobe-Korea1, Mac KS X 1001:1992 EUC-KR encoding, horizontal.
    KscpcEucH,

    // ── Adobe-CNS1 (Traditional Chinese) ── Unicode encodings ──
    /// `UniCNS-UCS2-H` — Adobe-CNS1, Unicode UCS-2 encoding, horizontal.
    UniCnsUcs2H,
    /// `UniCNS-UCS2-V` — Adobe-CNS1, Unicode UCS-2 encoding, vertical.
    UniCnsUcs2V,
    /// `UniCNS-UTF16-H` — Adobe-CNS1, Unicode UTF-16 encoding, horizontal.
    UniCnsUtf16H,
    /// `UniCNS-UTF16-V` — Adobe-CNS1, Unicode UTF-16 encoding, vertical.
    UniCnsUtf16V,

    // ── Adobe-GB1 (Simplified Chinese) ── Unicode encodings ──
    /// `UniGB-UCS2-H` — Adobe-GB1, Unicode UCS-2 encoding, horizontal.
    UniGbUcs2H,
    /// `UniGB-UCS2-V` — Adobe-GB1, Unicode UCS-2 encoding, vertical.
    UniGbUcs2V,
    /// `UniGB-UTF16-H` — Adobe-GB1, Unicode UTF-16 encoding, horizontal.
    UniGbUtf16H,
    /// `UniGB-UTF16-V` — Adobe-GB1, Unicode UTF-16 encoding, vertical.
    UniGbUtf16V,

    // ── Adobe-Japan1 (Japanese) ── Unicode encodings ──
    /// `UniJIS-UCS2-H` — Adobe-Japan1, Unicode UCS-2 encoding, horizontal.
    UniJisUcs2H,
    /// `UniJIS-UCS2-HW-H` — Adobe-Japan1, Unicode UCS-2 with half-width Roman, horizontal.
    UniJisUcs2HwH,
    /// `UniJIS-UCS2-HW-V` — Adobe-Japan1, Unicode UCS-2 with half-width Roman, vertical.
    UniJisUcs2HwV,
    /// `UniJIS-UCS2-V` — Adobe-Japan1, Unicode UCS-2 encoding, vertical.
    UniJisUcs2V,
    /// `UniJIS-UTF16-H` — Adobe-Japan1, Unicode UTF-16 encoding, horizontal.
    UniJisUtf16H,
    /// `UniJIS-UTF16-V` — Adobe-Japan1, Unicode UTF-16 encoding, vertical.
    UniJisUtf16V,

    // ── Adobe-Korea1 (Korean) ── Unicode encodings ──
    /// `UniKS-UCS2-H` — Adobe-Korea1, Unicode UCS-2 encoding, horizontal.
    UniKsUcs2H,
    /// `UniKS-UCS2-V` — Adobe-Korea1, Unicode UCS-2 encoding, vertical.
    UniKsUcs2V,
    /// `UniKS-UTF16-H` — Adobe-Korea1, Unicode UTF-16 encoding, horizontal.
    UniKsUtf16H,
    /// `UniKS-UTF16-V` — Adobe-Korea1, Unicode UTF-16 encoding, vertical.
    UniKsUtf16V,

    // ── Adobe-Japan1 (Japanese) ── JIS encoding ──
    /// `V` — Adobe-Japan1, JIS X 0208 row-cell encoding, vertical.
    V,

    /// A custom (non-predefined) `CMap` name.
    Custom(&'a [u8]),
}

impl<'a> CMapName<'a> {
    /// Create a `CMapType` from raw bytes.
    pub fn from_bytes(name: &'a [u8]) -> Self {
        match name {
            b"83pv-RKSJ-H" => Self::N83pvRksjH,
            b"90ms-RKSJ-H" => Self::N90msRksjH,
            b"90ms-RKSJ-V" => Self::N90msRksjV,
            b"90msp-RKSJ-H" => Self::N90mspRksjH,
            b"90msp-RKSJ-V" => Self::N90mspRksjV,
            b"90pv-RKSJ-H" => Self::N90pvRksjH,
            b"Add-RKSJ-H" => Self::AddRksjH,
            b"Add-RKSJ-V" => Self::AddRksjV,
            b"B5pc-H" => Self::B5pcH,
            b"B5pc-V" => Self::B5pcV,
            b"CNS-EUC-H" => Self::CnsEucH,
            b"CNS-EUC-V" => Self::CnsEucV,
            b"ETen-B5-H" => Self::ETenB5H,
            b"ETen-B5-V" => Self::ETenB5V,
            b"ETenms-B5-H" => Self::ETenmsB5H,
            b"ETenms-B5-V" => Self::ETenmsB5V,
            b"EUC-H" => Self::EucH,
            b"EUC-V" => Self::EucV,
            b"Ext-RKSJ-H" => Self::ExtRksjH,
            b"Ext-RKSJ-V" => Self::ExtRksjV,
            b"GB-EUC-H" => Self::GbEucH,
            b"GB-EUC-V" => Self::GbEucV,
            b"GBK-EUC-H" => Self::GbkEucH,
            b"GBK-EUC-V" => Self::GbkEucV,
            b"GBK2K-H" => Self::Gbk2kH,
            b"GBK2K-V" => Self::Gbk2kV,
            b"GBKp-EUC-H" => Self::GbkpEucH,
            b"GBKp-EUC-V" => Self::GbkpEucV,
            b"GBpc-EUC-H" => Self::GbpcEucH,
            b"GBpc-EUC-V" => Self::GbpcEucV,
            b"H" => Self::H,
            b"HKscs-B5-H" => Self::HKscsB5H,
            b"HKscs-B5-V" => Self::HKscsB5V,
            b"Identity-H" => Self::IdentityH,
            b"Identity-V" => Self::IdentityV,
            b"KSC-EUC-H" => Self::KscEucH,
            b"KSC-EUC-V" => Self::KscEucV,
            b"KSCms-UHC-H" => Self::KscmsUhcH,
            b"KSCms-UHC-HW-H" => Self::KscmsUhcHwH,
            b"KSCms-UHC-HW-V" => Self::KscmsUhcHwV,
            b"KSCms-UHC-V" => Self::KscmsUhcV,
            b"KSCpc-EUC-H" => Self::KscpcEucH,
            b"UniCNS-UCS2-H" => Self::UniCnsUcs2H,
            b"UniCNS-UCS2-V" => Self::UniCnsUcs2V,
            b"UniCNS-UTF16-H" => Self::UniCnsUtf16H,
            b"UniCNS-UTF16-V" => Self::UniCnsUtf16V,
            b"UniGB-UCS2-H" => Self::UniGbUcs2H,
            b"UniGB-UCS2-V" => Self::UniGbUcs2V,
            b"UniGB-UTF16-H" => Self::UniGbUtf16H,
            b"UniGB-UTF16-V" => Self::UniGbUtf16V,
            b"UniJIS-UCS2-H" => Self::UniJisUcs2H,
            b"UniJIS-UCS2-HW-H" => Self::UniJisUcs2HwH,
            b"UniJIS-UCS2-HW-V" => Self::UniJisUcs2HwV,
            b"UniJIS-UCS2-V" => Self::UniJisUcs2V,
            b"UniJIS-UTF16-H" => Self::UniJisUtf16H,
            b"UniJIS-UTF16-V" => Self::UniJisUtf16V,
            b"UniKS-UCS2-H" => Self::UniKsUcs2H,
            b"UniKS-UCS2-V" => Self::UniKsUcs2V,
            b"UniKS-UTF16-H" => Self::UniKsUtf16H,
            b"UniKS-UTF16-V" => Self::UniKsUtf16V,
            b"V" => Self::V,
            _ => Self::Custom(name),
        }
    }

    /// Convert the `CMapType` back to its raw byte representation.
    pub fn to_bytes(&self) -> &[u8] {
        match self {
            Self::N83pvRksjH => b"83pv-RKSJ-H",
            Self::N90msRksjH => b"90ms-RKSJ-H",
            Self::N90msRksjV => b"90ms-RKSJ-V",
            Self::N90mspRksjH => b"90msp-RKSJ-H",
            Self::N90mspRksjV => b"90msp-RKSJ-V",
            Self::N90pvRksjH => b"90pv-RKSJ-H",
            Self::AddRksjH => b"Add-RKSJ-H",
            Self::AddRksjV => b"Add-RKSJ-V",
            Self::B5pcH => b"B5pc-H",
            Self::B5pcV => b"B5pc-V",
            Self::CnsEucH => b"CNS-EUC-H",
            Self::CnsEucV => b"CNS-EUC-V",
            Self::ETenB5H => b"ETen-B5-H",
            Self::ETenB5V => b"ETen-B5-V",
            Self::ETenmsB5H => b"ETenms-B5-H",
            Self::ETenmsB5V => b"ETenms-B5-V",
            Self::EucH => b"EUC-H",
            Self::EucV => b"EUC-V",
            Self::ExtRksjH => b"Ext-RKSJ-H",
            Self::ExtRksjV => b"Ext-RKSJ-V",
            Self::GbEucH => b"GB-EUC-H",
            Self::GbEucV => b"GB-EUC-V",
            Self::GbkEucH => b"GBK-EUC-H",
            Self::GbkEucV => b"GBK-EUC-V",
            Self::Gbk2kH => b"GBK2K-H",
            Self::Gbk2kV => b"GBK2K-V",
            Self::GbkpEucH => b"GBKp-EUC-H",
            Self::GbkpEucV => b"GBKp-EUC-V",
            Self::GbpcEucH => b"GBpc-EUC-H",
            Self::GbpcEucV => b"GBpc-EUC-V",
            Self::H => b"H",
            Self::HKscsB5H => b"HKscs-B5-H",
            Self::HKscsB5V => b"HKscs-B5-V",
            Self::IdentityH => b"Identity-H",
            Self::IdentityV => b"Identity-V",
            Self::KscEucH => b"KSC-EUC-H",
            Self::KscEucV => b"KSC-EUC-V",
            Self::KscmsUhcH => b"KSCms-UHC-H",
            Self::KscmsUhcHwH => b"KSCms-UHC-HW-H",
            Self::KscmsUhcHwV => b"KSCms-UHC-HW-V",
            Self::KscmsUhcV => b"KSCms-UHC-V",
            Self::KscpcEucH => b"KSCpc-EUC-H",
            Self::UniCnsUcs2H => b"UniCNS-UCS2-H",
            Self::UniCnsUcs2V => b"UniCNS-UCS2-V",
            Self::UniCnsUtf16H => b"UniCNS-UTF16-H",
            Self::UniCnsUtf16V => b"UniCNS-UTF16-V",
            Self::UniGbUcs2H => b"UniGB-UCS2-H",
            Self::UniGbUcs2V => b"UniGB-UCS2-V",
            Self::UniGbUtf16H => b"UniGB-UTF16-H",
            Self::UniGbUtf16V => b"UniGB-UTF16-V",
            Self::UniJisUcs2H => b"UniJIS-UCS2-H",
            Self::UniJisUcs2HwH => b"UniJIS-UCS2-HW-H",
            Self::UniJisUcs2HwV => b"UniJIS-UCS2-HW-V",
            Self::UniJisUcs2V => b"UniJIS-UCS2-V",
            Self::UniJisUtf16H => b"UniJIS-UTF16-H",
            Self::UniJisUtf16V => b"UniJIS-UTF16-V",
            Self::UniKsUcs2H => b"UniKS-UCS2-H",
            Self::UniKsUcs2V => b"UniKS-UCS2-V",
            Self::UniKsUtf16H => b"UniKS-UTF16-H",
            Self::UniKsUtf16V => b"UniKS-UTF16-V",
            Self::V => b"V",
            Self::Custom(name) => name,
        }
    }
}

/// Let's limit the number of nested `usecmap` references to 16.
const MAX_NESTING_DEPTH: u32 = 16;

/// A parsed cmap.
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
    /// Parse a cmap from raw bytes.
    ///
    /// The `get_cmap` callback is used to recursively resolve cmaps that
    /// are referenced via `usecmap`.
    pub fn parse<'a>(
        data: &[u8],
        get_cmap: impl Fn(CMapName<'_>) -> Option<&'a [u8]> + Clone + 'a,
    ) -> Option<Self> {
        parse::parse_inner(data, get_cmap, 0)
    }

    /// Create an Identity-H cmap.
    pub fn identity_h() -> Self {
        Self::identity(WritingMode::Horizontal, b"Identity-H")
    }

    /// Create an Identity-V cmap.
    pub fn identity_v() -> Self {
        Self::identity(WritingMode::Vertical, b"Identity-V")
    }

    fn identity(writing_mode: WritingMode, name: &[u8]) -> Self {
        Self {
            metadata: Metadata {
                character_collection: Some(CharacterCollection {
                    family: CidFamily::AdobeIdentity,
                    supplement: 0,
                }),
                name: Some(Vec::from(name)),
                writing_mode: Some(writing_mode),
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

    /// Return the metadata of this cmap.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Look up the CID code of a character code.
    ///
    /// Returns `None` if the code is not within any codespace range for the
    /// given byte length.
    pub fn lookup_cid_code(&self, code: u32, byte_len: u8) -> Option<Cid> {
        let in_codespace = self.in_codespace(code, byte_len);

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

        // See pdfjs_bug920426, it uses bf chars for encoding the characters
        // of a text-showing operator, so try that as well. We don't want to
        // recurse though, only check the bf chars of this level.
        if let Some(UnicodeString::Char(lookup)) = self.lookup_unicode_code_inner(code, false) {
            return Some(lookup as u32);
        }

        // If character code is in code space range but has no active mapping,
        // here or in any referenced cmaps, assume `.notdef`.
        Some(
            self.base
                .as_ref()
                .and_then(|b| b.lookup_cid_code(code, byte_len))
                .unwrap_or(0),
        )
    }

    /// Look up the base font code of the given character code. This is usually
    /// used for `ToUnicode` cmaps
    ///
    /// Returns `None` if no mapping is available.
    pub fn lookup_unicode_code(&self, code: u32) -> Option<UnicodeString> {
        self.lookup_unicode_code_inner(code, true)
    }

    /// Check whether a character code is within any codespace range, including
    /// those of the base cmap (for cmap files that inherit codespace via `usecmap`).
    fn in_codespace(&self, code: u32, byte_len: u8) -> bool {
        if !self.codespace_ranges.is_empty() {
            return self
                .codespace_ranges
                .iter()
                .any(|r| r.number_bytes == byte_len && code >= r.low && code <= r.high);
        }

        // If nothing was found in this cmap, check if there's a parent cmap.
        self.base
            .as_ref()
            .is_some_and(|b| b.in_codespace(code, byte_len))
    }

    fn lookup_unicode_code_inner(&self, code: u32, recurse: bool) -> Option<UnicodeString> {
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

        if recurse {
            self.base.as_ref()?.lookup_unicode_code(code)
        } else {
            None
        }
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

#[derive(Debug, Clone)]
pub(crate) struct BfRange {
    pub(crate) range: Range,
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

/// A Unicode value decoded from a cmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnicodeString {
    /// A single Unicode character.
    Char(char),
    /// A string consisting of multiple Unicode characters, stored as a UTF-8 string.
    String(String),
}

/// Metadata extracted from a cmap file.
#[derive(Debug, Clone)]
pub struct Metadata {
    /// The referenced character collection.
    pub character_collection: Option<CharacterCollection>,
    /// The cmap name.
    pub name: Option<Vec<u8>>,
    /// The writing mode.
    pub writing_mode: Option<WritingMode>,
}

/// The registry+ordering family of a CID character collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CidFamily {
    /// Adobe-Japan1
    AdobeJapan1,
    /// Adobe-GB1
    AdobeGB1,
    /// Adobe-CNS1
    AdobeCNS1,
    /// Adobe-Korea1
    AdobeKorea1,
    /// Adobe-Identity
    AdobeIdentity,
    /// A non-predefined registry/ordering pair.
    Custom {
        /// The registry name.
        registry: Vec<u8>,
        /// The ordering name.
        ordering: Vec<u8>,
    },
}

impl CidFamily {
    /// Create a `CidFamily` from raw registry and ordering byte strings.
    pub fn from_registry_ordering(registry: &[u8], ordering: &[u8]) -> Self {
        if registry == b"Adobe" {
            match ordering {
                b"Japan1" => Self::AdobeJapan1,
                b"GB1" => Self::AdobeGB1,
                b"CNS1" => Self::AdobeCNS1,
                b"Korea1" => Self::AdobeKorea1,
                b"Identity" => Self::AdobeIdentity,
                _ => Self::Custom {
                    registry: registry.to_vec(),
                    ordering: ordering.to_vec(),
                },
            }
        } else {
            Self::Custom {
                registry: registry.to_vec(),
                ordering: ordering.to_vec(),
            }
        }
    }
}

/// A CID character collection identifying the character set and ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterCollection {
    /// The registry+ordering family.
    pub family: CidFamily,
    /// The supplement number.
    pub supplement: i32,
}

/// The writing mode of a cmap.
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

    // Note that those cmaps might not be completely valid according to the rules
    // of cmap/Postscript, but since our parser is very lenient and doesn't run a real
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
        assert_eq!(cc.family, CidFamily::AdobeJapan1);
        assert_eq!(cc.supplement, 6);
        assert_eq!(
            cmap.metadata().name.as_deref(),
            Some(b"Adobe-Japan1-H".as_slice())
        );
        assert_eq!(cmap.metadata().writing_mode, Some(WritingMode::Horizontal));
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
        assert_eq!(cmap.metadata().writing_mode, Some(WritingMode::Vertical));
        assert_eq!(
            cmap.metadata().name.as_deref(),
            Some(b"Adobe-Japan1-V".as_slice())
        );
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
    fn dict_style_cidsysteminfo() {
        let data = br#"
/CIDInit /ProcSet findresource begin
10 dict begin
begincmap
/CIDSystemInfo
<< /Registry (Adobe)
/Ordering (UCS)
/Supplement 0
>> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
2 beginbfchar
<001F> <F049>
<002A> <F055>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end
"#;
        let cmap = CMap::parse(data, |_| None).unwrap();
        let cc = cmap.metadata().character_collection.as_ref().unwrap();
        assert_eq!(
            cc.family,
            CidFamily::Custom {
                registry: b"Adobe".to_vec(),
                ordering: b"UCS".to_vec(),
            }
        );
        assert_eq!(cc.supplement, 0);
        assert_eq!(
            cmap.metadata().name.as_deref(),
            Some(b"Adobe-Identity-UCS".as_slice())
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x001F),
            Some(UnicodeString::Char('\u{F049}'))
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x002A),
            Some(UnicodeString::Char('\u{F055}'))
        );
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
            if name.to_bytes() == b"Base" {
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
            if name.to_bytes() == b"Base" {
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
        assert_eq!(
            cmap.metadata().name.as_deref(),
            Some(b"Identity-H".as_slice())
        );
        assert_eq!(cmap.metadata().writing_mode, Some(WritingMode::Horizontal));

        assert_eq!(cmap.lookup_cid_code(0x0041, 2), Some(0x0041));
        assert_eq!(cmap.lookup_cid_code(0x1234, 2), Some(0x1234));
        assert_eq!(cmap.lookup_cid_code(0xFFFF, 2), Some(0xFFFF));

        assert_eq!(cmap.lookup_cid_code(0x0041, 1), None);
        assert_eq!(cmap.lookup_cid_code(0x0041, 3), None);
    }

    #[test]
    fn identity_v() {
        let cmap = CMap::identity_v();
        assert_eq!(
            cmap.metadata().name.as_deref(),
            Some(b"Identity-V".as_slice())
        );
        assert_eq!(cmap.metadata().writing_mode, Some(WritingMode::Vertical));

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

    #[test]
    fn minimal_cmap_no_name_no_wmode() {
        // Extracted from corpus PDF 0500013.
        let data = br#"
/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CMapType 2 def
1 begincodespacerange
<00><ff>
endcodespacerange
1 beginbfrange
<01><01><0020>
endbfrange
endcmap
"#;
        let cmap = CMap::parse(data, |_| None).unwrap();
        assert_eq!(cmap.metadata().name, None);
        assert_eq!(cmap.metadata().character_collection, None);
        assert_eq!(cmap.metadata().writing_mode, None);
        assert_eq!(
            cmap.lookup_unicode_code(0x01),
            Some(UnicodeString::Char(' '))
        );
    }

    #[test]
    fn registry_as_name() {
        // Extracted from corpus PDF 0875241, uses names instead of strings for
        // strings for metadata.
        let data = br#"
/CIDSystemInfo
<< /Registry /ABCDEF+SimSun
/Ordering (pdfbeaninc)
/Supplement 0
>> def
/CMapName /ABCDEF+SimSun def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 beginbfrange
<0000> <0000> <6881>
endbfrange
"#;
        let cmap = CMap::parse(data, |_| None).unwrap();
        let cc = cmap.metadata().character_collection.as_ref().unwrap();
        assert_eq!(
            cc.family,
            CidFamily::Custom {
                registry: b"ABCDEF+SimSun".to_vec(),
                ordering: b"pdfbeaninc".to_vec(),
            }
        );
        assert_eq!(cc.supplement, 0);
        assert_eq!(
            cmap.lookup_unicode_code(0x0000),
            Some(UnicodeString::Char('\u{6881}'))
        );
    }

    #[test]
    fn single_byte_bfrange_destination() {
        // See for example corpus test case 0625248.
        let cmap = parse_with_preamble(
            br#"
1 beginbfrange
<00> <7f> <00>
endbfrange
"#,
        );

        assert_eq!(
            cmap.lookup_unicode_code(0x00),
            Some(UnicodeString::Char('\u{0000}'))
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x41),
            Some(UnicodeString::Char('A'))
        );
        assert_eq!(
            cmap.lookup_unicode_code(0x7f),
            Some(UnicodeString::Char('\u{007F}'))
        );
    }
}

#[cfg(all(test, feature = "embed-cmaps"))]
mod bcmap_tests {
    use super::*;

    fn get_embedded_cmap(name: CMapName<'_>) -> Option<&'static [u8]> {
        load_embedded(name)
    }

    #[test]
    fn embedded_h_cmap() {
        let data = load_embedded(CMapName::H).expect("embedded H cmap not found");
        let cmap = CMap::parse(data, get_embedded_cmap).expect("failed to parse H cmap");
        assert_eq!(cmap.metadata().writing_mode, None);
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeJapan1,
                supplement: 1,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x2121, 2), Some(633));
        assert_eq!(cmap.lookup_cid_code(0x2221, 2), Some(727));
    }

    #[test]
    fn embedded_90ms_rksj_h() {
        let data =
            load_embedded(CMapName::N90msRksjH).expect("embedded 90ms-RKSJ-H cmap not found");
        let cmap = CMap::parse(data, get_embedded_cmap).expect("failed to parse 90ms-RKSJ-H cmap");
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeJapan1,
                supplement: 2,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x20, 1), Some(231));
        assert_eq!(cmap.lookup_cid_code(0x8140, 2), Some(633));
    }

    #[test]
    fn embedded_v_cmap() {
        let data = load_embedded(CMapName::V).expect("embedded V cmap not found");
        let cmap = CMap::parse(data, get_embedded_cmap).expect("failed to parse V cmap");
        assert_eq!(cmap.metadata().writing_mode, Some(WritingMode::Vertical));
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeJapan1,
                supplement: 1,
            })
        );
        // Inherited from base "H" via usecmap.
        assert_eq!(cmap.lookup_cid_code(0x2121, 2), Some(633));
    }

    #[test]
    fn embedded_gbk_euc_h() {
        let data = load_embedded(CMapName::GbkEucH).expect("embedded GBK-EUC-H not found");
        let cmap = CMap::parse(data, get_embedded_cmap).expect("failed to parse GBK-EUC-H");
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeGB1,
                supplement: 2,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x20, 1), Some(7716));
        assert_eq!(cmap.lookup_cid_code(0x21, 1), Some(814));
        assert_eq!(cmap.lookup_cid_code(0x7E, 1), Some(814 + 0x7E - 0x21));
        assert_eq!(cmap.lookup_cid_code(0xFE80, 2), Some(22094));
        assert_eq!(cmap.lookup_cid_code(0xFEA0, 2), Some(22094 + 0xA0 - 0x80));
    }

    #[test]
    fn embedded_ksc_euc_h() {
        let data = load_embedded(CMapName::KscEucH).unwrap();
        let cmap = CMap::parse(data, get_embedded_cmap).unwrap();
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeKorea1,
                supplement: 0,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x20, 1), Some(8094));
        assert_eq!(cmap.lookup_cid_code(0x41, 1), Some(8094 + 0x41 - 0x20));
        assert_eq!(cmap.lookup_cid_code(0xA1A1, 2), Some(101));
        assert_eq!(cmap.lookup_cid_code(0xA1FE, 2), Some(101 + 0xFE - 0xA1));
        assert_eq!(cmap.lookup_cid_code(0xFDA1, 2), Some(7962));
    }

    #[test]
    fn embedded_b5pc_h() {
        let data = load_embedded(CMapName::B5pcH).unwrap();
        let cmap = CMap::parse(data, get_embedded_cmap).unwrap();
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeCNS1,
                supplement: 0,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x20, 1), Some(1));
        assert_eq!(cmap.lookup_cid_code(0x41, 1), Some(1 + 0x41 - 0x20));
        assert_eq!(cmap.lookup_cid_code(0x80, 1), Some(61));
        assert_eq!(cmap.lookup_cid_code(0xD140, 2), Some(7251));
        assert_eq!(cmap.lookup_cid_code(0xD17E, 2), Some(7251 + 0x7E - 0x40));
        assert_eq!(cmap.lookup_cid_code(0xF9D5, 2), Some(13642 + 3));
    }

    #[test]
    fn embedded_unijis_utf16_h() {
        let data = load_embedded(CMapName::UniJisUtf16H).unwrap();
        let cmap = CMap::parse(data, get_embedded_cmap).unwrap();
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeJapan1,
                supplement: 7,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x20, 2), Some(1));
        assert_eq!(cmap.lookup_cid_code(0x41, 2), Some(1 + 0x41 - 0x20));
        assert_eq!(cmap.lookup_cid_code(0x5C, 2), Some(97));
        assert_eq!(cmap.lookup_cid_code(0x5BCC, 2), Some(3531));
        assert_eq!(cmap.lookup_cid_code(0xD884DF50, 4), Some(19130));
    }

    #[test]
    fn embedded_identity_h() {
        let data = load_embedded(CMapName::IdentityH).expect("embedded Identity-H not found");
        let cmap = CMap::parse(data, get_embedded_cmap).expect("failed to parse Identity-H cmap");
        assert_eq!(cmap.metadata().writing_mode, None);
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeIdentity,
                supplement: 0,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x0041, 2), Some(0x41));
        assert_eq!(cmap.lookup_cid_code(0xFFFF, 2), Some(0xFFFF));
    }

    #[test]
    fn embedded_identity_v() {
        let data = load_embedded(CMapName::IdentityV).expect("embedded Identity-V not found");
        let cmap = CMap::parse(data, get_embedded_cmap).expect("failed to parse Identity-V cmap");
        assert_eq!(cmap.metadata().writing_mode, Some(WritingMode::Vertical));
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeIdentity,
                supplement: 0,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x0000, 2), Some(0));
        assert_eq!(cmap.lookup_cid_code(0x0041, 2), Some(0x41));
        assert_eq!(cmap.lookup_cid_code(0xFFFF, 2), Some(0xFFFF));
    }

    #[test]
    fn embedded_eten_b5_h() {
        let data = load_embedded(CMapName::ETenB5H).unwrap();
        let cmap = CMap::parse(data, get_embedded_cmap).unwrap();
        assert_eq!(
            cmap.metadata().character_collection,
            Some(CharacterCollection {
                family: CidFamily::AdobeCNS1,
                supplement: 0,
            })
        );
        assert_eq!(cmap.lookup_cid_code(0x20, 1), Some(13648));
        assert_eq!(cmap.lookup_cid_code(0x7E, 1), Some(13648 + 0x7E - 0x20));
        assert_eq!(cmap.lookup_cid_code(0xA140, 2), Some(99));
        assert_eq!(cmap.lookup_cid_code(0xA158, 2), Some(99 + 0x58 - 0x40));
        assert_eq!(cmap.lookup_cid_code(0xD040, 2), Some(7094));
        assert_eq!(cmap.lookup_cid_code(0xF9FE, 2), Some(14056 + 0xFE - 0xD6));
    }
}
