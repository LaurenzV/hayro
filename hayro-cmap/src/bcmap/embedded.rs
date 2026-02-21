use std::ops::Range;
use std::sync::LazyLock;

use super::huffman::{self, HuffmanTable};
use super::reader::Reader;
use crate::CMapName;

pub(super) static BUNDLE: LazyLock<Bundle> = LazyLock::new(|| {
    // We already know the bundle is valid, so we can skip validation and just
    // unwrap everywhere.

    let compressed = include_bytes!("../../assets/cmaps.brotli");
    let mut decompressed = Vec::new();
    let mut reader = compressed.as_slice();

    brotli::BrotliDecompress(&mut reader, &mut decompressed)
        .ok()
        .unwrap();

    let mut reader = Reader::new(&decompressed);
    let huff_size = reader.read_u32().unwrap() as usize;
    let huff_data = reader.read_bytes(huff_size).unwrap();
    let (delta_table, count_table) = huffman::decode_tables(huff_data).unwrap();

    let mut entries = Vec::new();

    while !reader.at_end() {
        let start = reader.position();

        // Skip file magic and version.
        reader.read_bytes(6).unwrap();
        let file_len = reader.read_u32().unwrap() as usize;

        reader.read_bytes(file_len - 10).unwrap();
        entries.push(start..start + file_len);
    }

    Bundle {
        data: decompressed,
        delta_table,
        count_table,
        entries,
    }
});

pub(super) struct Bundle {
    data: Vec<u8>,
    pub(super) delta_table: HuffmanTable,
    pub(super) count_table: HuffmanTable,
    entries: Vec<Range<usize>>,
}

/// Load the data for a cmap file, by name.
pub fn load_embedded(name: CMapName<'_>) -> Option<&'static [u8]> {
    // Get the index of the font of the cmap in the bundle. They are sorted
    // alphabetically.
    let idx = match name {
        CMapName::N83pvRksjH => 0,
        CMapName::N90msRksjH => 1,
        CMapName::N90msRksjV => 2,
        CMapName::N90mspRksjH => 3,
        CMapName::N90mspRksjV => 4,
        CMapName::N90pvRksjH => 5,
        CMapName::AddRksjH => 6,
        CMapName::AddRksjV => 7,
        CMapName::AdobeCns1Ucs2 => 8,
        CMapName::AdobeGb1Ucs2 => 9,
        CMapName::AdobeJapan1Ucs2 => 10,
        CMapName::AdobeKorea1Ucs2 => 11,
        CMapName::B5pcH => 12,
        CMapName::B5pcV => 13,
        CMapName::CnsEucH => 14,
        CMapName::CnsEucV => 15,
        CMapName::ETenB5H => 16,
        CMapName::ETenB5V => 17,
        CMapName::ETenmsB5H => 18,
        CMapName::ETenmsB5V => 19,
        CMapName::EucH => 20,
        CMapName::EucV => 21,
        CMapName::ExtRksjH => 22,
        CMapName::ExtRksjV => 23,
        CMapName::GbEucH => 24,
        CMapName::GbEucV => 25,
        CMapName::GbkEucH => 26,
        CMapName::GbkEucV => 27,
        CMapName::Gbk2kH => 28,
        CMapName::Gbk2kV => 29,
        CMapName::GbkpEucH => 30,
        CMapName::GbkpEucV => 31,
        CMapName::GbpcEucH => 32,
        CMapName::GbpcEucV => 33,
        CMapName::H => 34,
        CMapName::HKscsB5H => 35,
        CMapName::HKscsB5V => 36,
        CMapName::IdentityH => 37,
        CMapName::IdentityV => 38,
        CMapName::KscEucH => 39,
        CMapName::KscEucV => 40,
        CMapName::KscmsUhcH => 41,
        CMapName::KscmsUhcHwH => 42,
        CMapName::KscmsUhcHwV => 43,
        CMapName::KscmsUhcV => 44,
        CMapName::KscpcEucH => 45,
        CMapName::UniCnsUcs2H => 46,
        CMapName::UniCnsUcs2V => 47,
        CMapName::UniCnsUtf16H => 48,
        CMapName::UniCnsUtf16V => 49,
        CMapName::UniGbUcs2H => 50,
        CMapName::UniGbUcs2V => 51,
        CMapName::UniGbUtf16H => 52,
        CMapName::UniGbUtf16V => 53,
        CMapName::UniJisUcs2H => 54,
        CMapName::UniJisUcs2HwH => 55,
        CMapName::UniJisUcs2HwV => 56,
        CMapName::UniJisUcs2V => 57,
        CMapName::UniJisUtf16H => 58,
        CMapName::UniJisUtf16V => 59,
        CMapName::UniKsUcs2H => 60,
        CMapName::UniKsUcs2V => 61,
        CMapName::UniKsUtf16H => 62,
        CMapName::UniKsUtf16V => 63,
        CMapName::V => 64,
        CMapName::Custom(_) => return None,
    };

    let range = BUNDLE.entries.get(idx)?;

    Some(&BUNDLE.data[range.clone()])
}
