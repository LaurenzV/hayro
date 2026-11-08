//! Writing a cmap back out as a PostScript `CMap` resource (Adobe TN 5014).
//!
//! A PostScript interpreter can then use the embedded cmaps as `/CMap`
//! resources (`composefont`, `findfont` of a CID-keyed font name).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use crate::{CMap, CidFamily, CidRange, WritingMode};

/// At most this many entries between `begincidrange` and `endcidrange`, as
/// TN 5014 requires.
const BLOCK: usize = 100;

impl CMap {
    /// This cmap as a PostScript `CMap` resource file: its codespace ranges,
    /// CID ranges and notdef ranges, `CIDSystemInfo` and `WMode`, with
    /// `usecmap` naming the cmap it builds on. Running the file defines the
    /// resource, as Adobe's own `CMap` files do.
    ///
    /// The Unicode (`bf`) entries of a `ToUnicode` cmap are not written: a
    /// PostScript `CMap` maps codes to CIDs.
    pub fn to_postscript(&self) -> Vec<u8> {
        let mut s = String::new();
        let name = self
            .metadata
            .name
            .as_deref()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_else(|| String::from("Unnamed"));
        let _ = writeln!(s, "%!PS-Adobe-3.0 Resource-CMap");
        let _ = writeln!(s, "%%DocumentNeededResources: ProcSet (CIDInit)");
        let _ = writeln!(s, "%%BeginResource: CMap ({name})");
        let _ = writeln!(s, "/CIDInit /ProcSet findresource begin");
        let _ = writeln!(s, "12 dict begin");
        let _ = writeln!(s, "begincmap");
        if let Some(base) = &self.base
            && let Some(base_name) = &base.metadata.name
        {
            let _ = writeln!(s, "/{} usecmap", String::from_utf8_lossy(base_name));
        }
        if let Some(cc) = &self.metadata.character_collection {
            let (registry, ordering): (&[u8], &[u8]) = match &cc.family {
                CidFamily::AdobeJapan1 => (b"Adobe", b"Japan1"),
                CidFamily::AdobeGB1 => (b"Adobe", b"GB1"),
                CidFamily::AdobeCNS1 => (b"Adobe", b"CNS1"),
                CidFamily::AdobeKorea1 => (b"Adobe", b"Korea1"),
                CidFamily::AdobeIdentity => (b"Adobe", b"Identity"),
                CidFamily::Custom { registry, ordering } => (registry, ordering),
            };
            let _ = writeln!(
                s,
                "/CIDSystemInfo 3 dict dup begin /Registry ({}) def /Ordering ({}) def /Supplement {} def end def",
                String::from_utf8_lossy(registry),
                String::from_utf8_lossy(ordering),
                cc.supplement
            );
        }
        let _ = writeln!(s, "/CMapName /{name} def");
        let _ = writeln!(s, "/CMapType 1 def");
        let wmode = match self.metadata.writing_mode {
            Some(WritingMode::Vertical) => 1,
            _ => 0,
        };
        let _ = writeln!(s, "/WMode {wmode} def");

        let hex = |v: u32, n: u8| {
            let mut h = String::new();
            for i in (0..n).rev() {
                let _ = write!(h, "{:02X}", (v >> (8 * u32::from(i))) & 0xFF);
            }
            h
        };

        for chunk in self._codespace_ranges.chunks(BLOCK) {
            let _ = writeln!(s, "{} begincodespacerange", chunk.len());
            for r in chunk {
                let _ = writeln!(
                    s,
                    "<{}> <{}>",
                    hex(r.low, r.number_bytes),
                    hex(r.high, r.number_bytes)
                );
            }
            let _ = writeln!(s, "endcodespacerange");
        }

        let ranges = |s: &mut String, which: &crate::PartitionedRanges, op: &str| {
            for n in 1..=4_u8 {
                let Some(entries) = which.get(n) else {
                    continue;
                };
                for chunk in entries.chunks(BLOCK) {
                    let _ = writeln!(s, "{} begin{op}", chunk.len());
                    for CidRange { range, cid_start } in chunk {
                        let _ = writeln!(
                            s,
                            "<{}> <{}> {cid_start}",
                            hex(range.start, n),
                            hex(range.end, n)
                        );
                    }
                    let _ = writeln!(s, "end{op}");
                }
            }
        };
        ranges(&mut s, &self.notdef_ranges, "notdefrange");
        ranges(&mut s, &self.cid_ranges, "cidrange");

        let _ = writeln!(s, "endcmap");
        let _ = writeln!(s, "CMapName currentdict /CMap defineresource pop");
        let _ = writeln!(s, "end");
        let _ = writeln!(s, "end");
        let _ = writeln!(s, "%%EndResource");
        let _ = writeln!(s, "%%EOF");
        s.into_bytes()
    }
}

#[cfg(all(test, feature = "embed-cmaps"))]
mod tests {
    use crate::{CMap, CMapName, load_embedded};

    #[test]
    fn rksj_round_trips_through_postscript() {
        let get = |n: CMapName<'_>| load_embedded(n);
        let cmap = CMap::parse(
            load_embedded(CMapName::from_bytes(b"90ms-RKSJ-V")).unwrap(),
            get,
        )
        .unwrap();
        let ps = cmap.to_postscript();
        let text = core::str::from_utf8(&ps).unwrap();
        assert!(text.contains("/90ms-RKSJ-H usecmap"), "{}", &text[..400]);
        assert!(text.contains("/WMode 1 def"));
        assert!(text.contains("/Ordering (Japan1)"));

        // Parsed back (with the base from the embedded set), every code maps as before.
        let back = CMap::parse(&ps, get).unwrap();
        let base = CMap::parse(
            load_embedded(CMapName::from_bytes(b"90ms-RKSJ-H")).unwrap(),
            get,
        )
        .unwrap();
        let base_back = CMap::parse(&base.to_postscript(), get).unwrap();
        for code in 0..=0xFFFF_u32 {
            let n = if code < 0x100 { 1 } else { 2 };
            assert_eq!(
                back.lookup_cid_code(code, n),
                cmap.lookup_cid_code(code, n),
                "{code:04X}"
            );
            assert_eq!(
                base_back.lookup_cid_code(code, n),
                base.lookup_cid_code(code, n),
                "{code:04X}"
            );
        }
    }
}
