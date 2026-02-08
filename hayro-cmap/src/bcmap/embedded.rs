use std::ops::Range;
use std::sync::LazyLock;

use super::huffman::{self, HuffmanTable};
use super::reader::Reader;

static BUNDLE: LazyLock<Option<Bundle>> = LazyLock::new(|| {
    let compressed = include_bytes!("../../assets/cmaps.brotli");
    let mut decompressed = Vec::new();
    let mut reader = compressed.as_slice();

    brotli::BrotliDecompress(&mut reader, &mut decompressed).ok()?;

    let mut reader = Reader::new(&decompressed);
    let huff_size = reader.read_u32()? as usize;
    let huff_data = reader.read_bytes(huff_size)?;
    let (delta_table, count_table) = huffman::decode_tables(huff_data)?;

    // Precompute byte ranges for each bcmap file in the decompressed data.
    let mut entries = Vec::new();

    while !reader.at_end() {
        let start = reader.position();

        if reader.read_bytes(5)? != b"bcmap" {
            break;
        }

        reader.read_u8()?; // version
        let file_len = reader.read_u32()? as usize;

        if file_len < 10 {
            break;
        }

        reader.read_bytes(file_len - 10)?;
        entries.push(start..start + file_len);
    }

    Some(Bundle {
        data: decompressed,
        delta_table,
        count_table,
        entries,
    })
});

struct Bundle {
    data: Vec<u8>,
    delta_table: HuffmanTable,
    count_table: HuffmanTable,
    entries: Vec<Range<usize>>,
}

/// Look up an embedded bcmap by `CMap` name, returning the raw bcmap bytes.
pub fn load_embedded(name: &[u8]) -> Option<&'static [u8]> {
    let bundle = (*BUNDLE).as_ref()?;
    let idx = cmap_index(name)?;
    let range = bundle.entries.get(idx)?;
    Some(&bundle.data[range.start..range.end])
}

/// Return references to the cached Huffman trees.
pub(super) fn huffman_trees() -> Option<(&'static HuffmanTable, &'static HuffmanTable)> {
    let bundle = (*BUNDLE).as_ref()?;
    Some((&bundle.delta_table, &bundle.count_table))
}

/// Map a predefined PDF `CMap` name to its index in the sorted bundle.
///
/// The bundle contains exactly the `CMaps` from ISO 32000-2 Table 116,
/// sorted alphabetically. This match must stay in sync with the set
/// in `cmap_common.py::PDF_PREDEFINED`.
fn cmap_index(name: &[u8]) -> Option<usize> {
    // Sorted alphabetically — indices 0..60.
    Some(match name {
        b"83pv-RKSJ-H" => 0,
        b"90ms-RKSJ-H" => 1,
        b"90ms-RKSJ-V" => 2,
        b"90msp-RKSJ-H" => 3,
        b"90msp-RKSJ-V" => 4,
        b"90pv-RKSJ-H" => 5,
        b"Add-RKSJ-H" => 6,
        b"Add-RKSJ-V" => 7,
        b"B5pc-H" => 8,
        b"B5pc-V" => 9,
        b"CNS-EUC-H" => 10,
        b"CNS-EUC-V" => 11,
        b"ETen-B5-H" => 12,
        b"ETen-B5-V" => 13,
        b"ETenms-B5-H" => 14,
        b"ETenms-B5-V" => 15,
        b"EUC-H" => 16,
        b"EUC-V" => 17,
        b"Ext-RKSJ-H" => 18,
        b"Ext-RKSJ-V" => 19,
        b"GB-EUC-H" => 20,
        b"GB-EUC-V" => 21,
        b"GBK-EUC-H" => 22,
        b"GBK-EUC-V" => 23,
        b"GBK2K-H" => 24,
        b"GBK2K-V" => 25,
        b"GBKp-EUC-H" => 26,
        b"GBKp-EUC-V" => 27,
        b"GBpc-EUC-H" => 28,
        b"GBpc-EUC-V" => 29,
        b"H" => 30,
        b"HKscs-B5-H" => 31,
        b"HKscs-B5-V" => 32,
        b"Identity-H" => 33,
        b"Identity-V" => 34,
        b"KSC-EUC-H" => 35,
        b"KSC-EUC-V" => 36,
        b"KSCms-UHC-H" => 37,
        b"KSCms-UHC-HW-H" => 38,
        b"KSCms-UHC-HW-V" => 39,
        b"KSCms-UHC-V" => 40,
        b"KSCpc-EUC-H" => 41,
        b"UniCNS-UCS2-H" => 42,
        b"UniCNS-UCS2-V" => 43,
        b"UniCNS-UTF16-H" => 44,
        b"UniCNS-UTF16-V" => 45,
        b"UniGB-UCS2-H" => 46,
        b"UniGB-UCS2-V" => 47,
        b"UniGB-UTF16-H" => 48,
        b"UniGB-UTF16-V" => 49,
        b"UniJIS-UCS2-H" => 50,
        b"UniJIS-UCS2-HW-H" => 51,
        b"UniJIS-UCS2-HW-V" => 52,
        b"UniJIS-UCS2-V" => 53,
        b"UniJIS-UTF16-H" => 54,
        b"UniJIS-UTF16-V" => 55,
        b"UniKS-UCS2-H" => 56,
        b"UniKS-UCS2-V" => 57,
        b"UniKS-UTF16-H" => 58,
        b"UniKS-UTF16-V" => 59,
        b"V" => 60,
        _ => return None,
    })
}
