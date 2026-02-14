use std::ops::Range;
use std::sync::LazyLock;

use super::huffman::{self, HuffmanTable};
use super::reader::Reader;
use crate::CMapType;

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
pub fn load_embedded(name: CMapType<'_>) -> Option<&'static [u8]> {
    // Get the index of the font of the cmap in the bundle. They are sorted
    // alphabetically.
    let idx = match name {
        CMapType::N83pvRksjH => 0,
        CMapType::N90msRksjH => 1,
        CMapType::N90msRksjV => 2,
        CMapType::N90mspRksjH => 3,
        CMapType::N90mspRksjV => 4,
        CMapType::N90pvRksjH => 5,
        CMapType::AddRksjH => 6,
        CMapType::AddRksjV => 7,
        CMapType::B5pcH => 8,
        CMapType::B5pcV => 9,
        CMapType::CnsEucH => 10,
        CMapType::CnsEucV => 11,
        CMapType::ETenB5H => 12,
        CMapType::ETenB5V => 13,
        CMapType::ETenmsB5H => 14,
        CMapType::ETenmsB5V => 15,
        CMapType::EucH => 16,
        CMapType::EucV => 17,
        CMapType::ExtRksjH => 18,
        CMapType::ExtRksjV => 19,
        CMapType::GbEucH => 20,
        CMapType::GbEucV => 21,
        CMapType::GbkEucH => 22,
        CMapType::GbkEucV => 23,
        CMapType::Gbk2kH => 24,
        CMapType::Gbk2kV => 25,
        CMapType::GbkpEucH => 26,
        CMapType::GbkpEucV => 27,
        CMapType::GbpcEucH => 28,
        CMapType::GbpcEucV => 29,
        CMapType::H => 30,
        CMapType::HKscsB5H => 31,
        CMapType::HKscsB5V => 32,
        CMapType::IdentityH => 33,
        CMapType::IdentityV => 34,
        CMapType::KscEucH => 35,
        CMapType::KscEucV => 36,
        CMapType::KscmsUhcH => 37,
        CMapType::KscmsUhcHwH => 38,
        CMapType::KscmsUhcHwV => 39,
        CMapType::KscmsUhcV => 40,
        CMapType::KscpcEucH => 41,
        CMapType::UniCnsUcs2H => 42,
        CMapType::UniCnsUcs2V => 43,
        CMapType::UniCnsUtf16H => 44,
        CMapType::UniCnsUtf16V => 45,
        CMapType::UniGbUcs2H => 46,
        CMapType::UniGbUcs2V => 47,
        CMapType::UniGbUtf16H => 48,
        CMapType::UniGbUtf16V => 49,
        CMapType::UniJisUcs2H => 50,
        CMapType::UniJisUcs2HwH => 51,
        CMapType::UniJisUcs2HwV => 52,
        CMapType::UniJisUcs2V => 53,
        CMapType::UniJisUtf16H => 54,
        CMapType::UniJisUtf16V => 55,
        CMapType::UniKsUcs2H => 56,
        CMapType::UniKsUcs2V => 57,
        CMapType::UniKsUtf16H => 58,
        CMapType::UniKsUtf16V => 59,
        CMapType::V => 60,
        CMapType::Custom(_) => return None,
    };

    let range = BUNDLE.entries.get(idx)?;

    Some(&BUNDLE.data[range.clone()])
}
