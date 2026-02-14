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
        CMapName::B5pcH => 8,
        CMapName::B5pcV => 9,
        CMapName::CnsEucH => 10,
        CMapName::CnsEucV => 11,
        CMapName::ETenB5H => 12,
        CMapName::ETenB5V => 13,
        CMapName::ETenmsB5H => 14,
        CMapName::ETenmsB5V => 15,
        CMapName::EucH => 16,
        CMapName::EucV => 17,
        CMapName::ExtRksjH => 18,
        CMapName::ExtRksjV => 19,
        CMapName::GbEucH => 20,
        CMapName::GbEucV => 21,
        CMapName::GbkEucH => 22,
        CMapName::GbkEucV => 23,
        CMapName::Gbk2kH => 24,
        CMapName::Gbk2kV => 25,
        CMapName::GbkpEucH => 26,
        CMapName::GbkpEucV => 27,
        CMapName::GbpcEucH => 28,
        CMapName::GbpcEucV => 29,
        CMapName::H => 30,
        CMapName::HKscsB5H => 31,
        CMapName::HKscsB5V => 32,
        CMapName::IdentityH => 33,
        CMapName::IdentityV => 34,
        CMapName::KscEucH => 35,
        CMapName::KscEucV => 36,
        CMapName::KscmsUhcH => 37,
        CMapName::KscmsUhcHwH => 38,
        CMapName::KscmsUhcHwV => 39,
        CMapName::KscmsUhcV => 40,
        CMapName::KscpcEucH => 41,
        CMapName::UniCnsUcs2H => 42,
        CMapName::UniCnsUcs2V => 43,
        CMapName::UniCnsUtf16H => 44,
        CMapName::UniCnsUtf16V => 45,
        CMapName::UniGbUcs2H => 46,
        CMapName::UniGbUcs2V => 47,
        CMapName::UniGbUtf16H => 48,
        CMapName::UniGbUtf16V => 49,
        CMapName::UniJisUcs2H => 50,
        CMapName::UniJisUcs2HwH => 51,
        CMapName::UniJisUcs2HwV => 52,
        CMapName::UniJisUcs2V => 53,
        CMapName::UniJisUtf16H => 54,
        CMapName::UniJisUtf16V => 55,
        CMapName::UniKsUcs2H => 56,
        CMapName::UniKsUcs2V => 57,
        CMapName::UniKsUtf16H => 58,
        CMapName::UniKsUtf16V => 59,
        CMapName::V => 60,
        CMapName::Custom(_) => return None,
    };

    let range = BUNDLE.entries.get(idx)?;

    Some(&BUNDLE.data[range.clone()])
}
