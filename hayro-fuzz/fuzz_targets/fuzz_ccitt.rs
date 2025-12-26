#![no_main]

use libfuzzer_sys::fuzz_target;

/// A decoder that simply discards all output.
struct Decoder;

impl hayro_ccitt::Decoder for Decoder {
    fn push_byte(&mut self, _byte: u8) {}
    fn push_bytes(&mut self, _byte: u8, _count: usize) {}
    fn next_line(&mut self) {}
}

// Header layout (10 bytes):
// [0..2]  columns (u16 LE)
// [2..4]  rows (u16 LE)
// [4]     end_of_block (bool)
// [5]     end_of_line (bool)
// [6]     rows_are_byte_aligned (bool)
// [7]     encoding_mode (0=Group4, 1=Group3_1D, 2+=Group3_2D)
// [8]     k parameter for Group3_2D
// [9]     invert_black (bool)
// [10..]  CCITT encoded data

const HEADER_SIZE: usize = 10;

fuzz_target!(|data: &[u8]| {
    if data.len() < HEADER_SIZE {
        return;
    }

    let columns = u16::from_le_bytes([data[0], data[1]]).max(1) as u32;
    let rows = u16::from_le_bytes([data[2], data[3]]).max(1) as u32;
    let end_of_block = data[4] != 0;
    let end_of_line = data[5] != 0;
    let rows_are_byte_aligned = data[6] != 0;
    let encoding = match data[7] % 3 {
        0 => hayro_ccitt::EncodingMode::Group4,
        1 => hayro_ccitt::EncodingMode::Group3_1D,
        _ => hayro_ccitt::EncodingMode::Group3_2D {
            k: data[8].max(1) as u32,
        },
    };
    let invert_black = data[9] != 0;

    let settings = hayro_ccitt::DecodeSettings {
        columns,
        rows,
        end_of_block,
        end_of_line,
        rows_are_byte_aligned,
        encoding,
        invert_black,
    };

    let mut decoder = Decoder;
    let _ = hayro_ccitt::decode(&data[HEADER_SIZE..], &mut decoder, &settings);
});
