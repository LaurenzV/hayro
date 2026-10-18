#![no_main]

use libfuzzer_sys::fuzz_target;

/// A decoder that simply discards all output.
struct Decoder;

impl hayro_ccitt::Decoder for Decoder {
    fn push_row(&mut self, _changes: &[u32]) {}
}

// Header layout (10 bytes):
// [0..2]  columns (u16 LE)
// [2..4]  rows (u16 LE)
// [4]     end_of_block (bool)
// [5]     end_of_line (bool)
// [6]     rows_are_byte_aligned (bool)
// [7]     encoding_mode (0=Group4, 1=Group3_1D, 2+=Group3_2D)
// [8]     unused
// [9]     unused
// [10..]  CCITT encoded data

const HEADER_SIZE: usize = 10;

fuzz_target!(|data: &[u8]| {
    if data.len() < HEADER_SIZE {
        return;
    }

    let columns = u16::from_le_bytes([data[0], data[1]]).max(1) as u32;
    let rows = u16::from_le_bytes([data[2], data[3]]).max(1) as u32;

    let mut settings = hayro_ccitt::DecodeSettings::new(columns, rows);
    settings.end_of_block = data[4] != 0;
    settings.end_of_line = data[5] != 0;
    settings.rows_are_byte_aligned = data[6] != 0;
    settings.encoding = match data[7] % 3 {
        0 => hayro_ccitt::EncodingMode::Group4,
        1 => hayro_ccitt::EncodingMode::Group3_1D,
        _ => hayro_ccitt::EncodingMode::Group3_2D,
    };

    let mut decoder = Decoder;
    let mut context = hayro_ccitt::DecoderContext::new(settings);
    let _ = hayro_ccitt::decode(&data[HEADER_SIZE..], &mut decoder, &mut context);
});
