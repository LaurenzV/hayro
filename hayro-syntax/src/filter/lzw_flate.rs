//! A decoder for LZW and flate-encoded streams.

use crate::bit_reader::{BitChunk, BitChunks, BitReader, BitSize, BitWriter};
use crate::object::dict::Dict;
use crate::object::dict::keys::{BITS_PER_COMPONENT, COLORS, COLUMNS, EARLY_CHANGE, PREDICTOR};
use itertools::izip;
use log::warn;

pub mod flate {
    use crate::filter::lzw_flate::{PredictorParams, apply_predictor};
    use crate::object::dict::Dict;
    use flate2::read::{DeflateDecoder, ZlibDecoder};
    use std::io::Read;

    /// Decode a flate-encoded stream.
    pub fn decode(data: &[u8], params: Dict) -> Option<Vec<u8>> {
        let decoded = zlib_stream(data).or_else(|| deflate_stream(data))?;
        let params = PredictorParams::from_params(&params);
        apply_predictor(decoded, &params)
    }

    fn zlib_stream(data: &[u8]) -> Option<Vec<u8>> {
        let mut decoder = ZlibDecoder::new(data);
        let mut result = Vec::new();

        decoder.read_to_end(&mut result).ok().map(|_| result)
    }

    fn deflate_stream(data: &[u8]) -> Option<Vec<u8>> {
        let mut decoder = DeflateDecoder::new(data);
        let mut result = Vec::new();

        decoder.read_to_end(&mut result).ok().map(|_| result)
    }
}

pub mod lzw {
    use crate::bit_reader::{BitReader, BitSize};
    use crate::filter::lzw_flate::{PredictorParams, apply_predictor};
    use crate::object::dict::Dict;
    use log::warn;

    /// Decode a LZW-encoded stream.
    pub fn decode(data: &[u8], params: Dict) -> Option<Vec<u8>> {
        let params = PredictorParams::from_params(&params);

        let decoded = decode_impl(data, params.early_change)?;

        apply_predictor(decoded, &params)
    }

    const CLEAR_TABLE: usize = 256;
    const EOD: usize = 257;
    const MAX_ENTRIES: usize = 4096;
    const INITIAL_SIZE: u16 = 258;

    fn decode_impl(data: &[u8], early_change: bool) -> Option<Vec<u8>> {
        let mut table = Table::new(early_change);
        let mut bit_size = BitSize::from_u8(table.code_length())?;
        let mut reader = BitReader::new(data);
        let mut decoded = vec![];
        let mut prev = None;

        loop {
            let next = match reader.read(bit_size) {
                Some(code) => code as usize,
                None => {
                    warn!("Premature EOF in LZW stream, EOD code missing");
                    return Some(decoded);
                }
            };

            match next {
                CLEAR_TABLE => {
                    table.clear();
                    prev = None;
                    bit_size = BitSize::from_u8(table.code_length())?;
                }
                EOD => return Some(decoded),
                new => {
                    if new > table.size() {
                        warn!("Invalid LZW code: {} (table size: {})", new, table.size());
                        return None;
                    }

                    if new < table.size() {
                        let entry = table.get(new)?;
                        let first_byte = entry[0];
                        decoded.extend_from_slice(entry);

                        if let Some(prev_code) = prev {
                            table.register(prev_code, first_byte);
                        }
                    } else if new == table.size() && prev.is_some() {
                        let prev_code = prev.unwrap();
                        let prev_entry = table.get(prev_code)?;
                        let first_byte = prev_entry[0];

                        let new_entry = table.register(prev_code, first_byte)?;
                        decoded.extend_from_slice(new_entry);
                    } else {
                        warn!("LZW decode error: code {} not found and prev is None", new);
                        return None;
                    }

                    bit_size = BitSize::from_u8(table.code_length())?;
                    prev = Some(new);
                }
            }
        }
    }

    struct Table {
        early_change: bool,
        entries: Vec<Option<Vec<u8>>>,
    }

    impl Table {
        fn new(early_change: bool) -> Self {
            let mut entries: Vec<_> = (0..=255).map(|b| Some(vec![b])).collect();

            // Clear table and EOD don't have any data.
            entries.push(None); // 256 = CLEAR_TABLE
            entries.push(None); // 257 = EOD

            Self {
                early_change,
                entries,
            }
        }

        fn push(&mut self, entry: Vec<u8>) -> Option<&[u8]> {
            if self.entries.len() >= MAX_ENTRIES {
                None
            } else {
                self.entries.push(Some(entry));
                self.entries.last()?.as_ref().map(|v| &**v)
            }
        }

        fn register(&mut self, prev: usize, new_byte: u8) -> Option<&[u8]> {
            let prev_entry = self.get(prev)?;

            let mut new_entry = Vec::with_capacity(prev_entry.len() + 1);
            new_entry.extend(prev_entry);
            new_entry.push(new_byte);
            self.push(new_entry)
        }

        fn get(&self, index: usize) -> Option<&[u8]> {
            self.entries.get(index)?.as_ref().map(|v| &**v)
        }

        fn clear(&mut self) {
            self.entries.truncate(INITIAL_SIZE as usize);
        }

        fn size(&self) -> usize {
            self.entries.len()
        }

        fn code_length(&self) -> u8 {
            const TEN: usize = 512;
            const ELEVEN: usize = 1024;
            const TWELVE: usize = 2048;

            let adjusted = self.entries.len() + (if self.early_change { 1 } else { 0 });

            if adjusted >= TWELVE {
                12
            } else if adjusted >= ELEVEN {
                11
            } else if adjusted >= TEN {
                10
            } else {
                9
            }
        }
    }
}

struct PredictorParams {
    predictor: u8,
    colors: u8,
    bits_per_component: u8,
    columns: usize,
    early_change: bool,
}

impl PredictorParams {
    fn bits_per_pixel(&self) -> u8 {
        self.bits_per_component * self.colors
    }

    fn bytes_per_pixel(&self) -> u8 {
        (self.bits_per_pixel() + 7) / 8
    }

    fn row_length_in_bytes(&self) -> Option<usize> {
        let raw = self.columns * self.bytes_per_pixel() as usize;

        match self.bits_per_component {
            // TODO: Find tests for 2,4,16 bits.
            1 => Some(raw.div_ceil(8)),
            2 => Some(raw.div_ceil(4)),
            4 => Some(raw.div_ceil(2)),
            8 => Some(raw),
            16 => Some(2 * raw),
            _ => {
                warn!("invalid bits per component {}", self.bits_per_component);

                None
            }
        }
    }
}

impl Default for PredictorParams {
    fn default() -> Self {
        Self {
            predictor: 1,
            colors: 1,
            bits_per_component: 8,
            columns: 1,
            early_change: true,
        }
    }
}

impl PredictorParams {
    fn from_params(dict: &Dict) -> Self {
        Self {
            predictor: dict.get(PREDICTOR).unwrap_or(1),
            colors: dict.get(COLORS).unwrap_or(1),
            bits_per_component: dict.get(BITS_PER_COMPONENT).unwrap_or(8),
            columns: dict.get(COLUMNS).unwrap_or(1),
            early_change: dict
                .get::<u8>(EARLY_CHANGE)
                .map(|e| if e == 0 { false } else { true })
                .unwrap_or(true),
        }
    }
}

fn apply_predictor(data: Vec<u8>, params: &PredictorParams) -> Option<Vec<u8>> {
    match params.predictor {
        1 => Some(data),
        i => {
            let is_png_predictor = i >= 10;

            let row_len = params.row_length_in_bytes()?;

            let total_row_len = if is_png_predictor {
                // + 1 Because each row must start with the predictor that is used for PNG predictors.
                row_len + 1
            } else {
                row_len
            };

            let num_rows = data.len() / total_row_len;

            if num_rows * total_row_len != data.len() {
                warn!("data length didn't match");

                return None;
            }

            if !matches!(params.bits_per_component, 1 | 2 | 4 | 8 | 16) {
                warn!("invalid bits per component {}", params.bits_per_component);

                return None;
            }

            let colors = params.colors as usize;
            let bit_size = BitSize::from_u8(params.bits_per_component)?;
            let zero_row = vec![0; row_len];
            let mut prev_row = BitChunks::new(&zero_row, bit_size, colors)?;
            let zero_col = BitChunk::new(0, colors);
            let mut out = vec![0; num_rows * row_len];
            let mut writer = BitWriter::new(&mut out, bit_size)?;

            for in_row in data.chunks_exact(total_row_len) {
                if is_png_predictor {
                    let predictor = in_row[0];
                    let in_data = &in_row[1..];
                    let in_data_chunks = BitChunks::new(in_data, bit_size, colors)?;

                    match predictor {
                        0 => {
                            // Just copy the data.
                            let mut reader = BitReader::new(in_data);

                            while let Some(data) = reader.read(bit_size) {
                                writer.write(data as u16);
                            }
                        }
                        1 => apply::<Sub>(
                            prev_row,
                            zero_col.clone(),
                            zero_col.clone(),
                            in_data_chunks,
                            &mut writer,
                            colors,
                            bit_size,
                        )?,
                        2 => apply::<Up>(
                            prev_row,
                            zero_col.clone(),
                            zero_col.clone(),
                            in_data_chunks,
                            &mut writer,
                            colors,
                            bit_size,
                        )?,
                        3 => apply::<Avg>(
                            prev_row,
                            zero_col.clone(),
                            zero_col.clone(),
                            in_data_chunks,
                            &mut writer,
                            colors,
                            bit_size,
                        )?,
                        4 => apply::<Paeth>(
                            prev_row,
                            zero_col.clone(),
                            zero_col.clone(),
                            in_data_chunks,
                            &mut writer,
                            colors,
                            bit_size,
                        )?,
                        n => {
                            warn!("invalid PNG predictor {}", n);

                            return None;
                        }
                    }
                } else if i == 2 {
                    apply::<Sub>(
                        prev_row,
                        zero_col.clone(),
                        zero_col.clone(),
                        BitChunks::new(in_row, bit_size, colors)?,
                        &mut writer,
                        colors,
                        bit_size,
                    );
                } else {
                    warn!("unknown predictor {}", i);

                    return None;
                }

                let (data, new_writer) = writer.split_off();
                writer = new_writer;
                prev_row = BitChunks::new(data, bit_size, colors)?;
            }

            Some(out)
        }
    }
}

fn apply<'a, T: Predictor>(
    prev_row: BitChunks<'a>,
    mut prev_col: BitChunk,
    mut top_left: BitChunk,
    cur_row: BitChunks<'a>,
    writer: &mut BitWriter<'a>,
    chunk_len: usize,
    bit_size: BitSize,
) -> Option<()> {
    for (cur_row, prev_row) in izip!(cur_row, prev_row) {
        let old_pos = writer.cur_pos();

        for (cur_row, prev_row, prev_col, top_left) in izip!(
            cur_row.iter(),
            prev_row.iter(),
            prev_col.iter(),
            top_left.iter()
        ) {
            // Note that the wrapping behavior when adding inside the predictors is dependent on the
            // bit size, so it wouldn't be triggered for bits per component < 16. So we mask out
            // the bytes manually, which is equivalent to a wrapping add.
            writer
                .write(T::predict(cur_row, prev_row, prev_col, top_left) & bit_size.mask() as u16);
        }

        prev_col = {
            let out_data = writer.get_data();
            let mut reader = BitReader::new_with(&out_data, old_pos);
            BitChunk::from_reader(&mut reader, bit_size, chunk_len).unwrap()
        };

        top_left = prev_row;
    }

    Some(())
}

trait Predictor {
    fn predict(cur_row: u16, prev_row: u16, prev_col: u16, top_left: u16) -> u16;
}

struct Sub;
impl Predictor for Sub {
    fn predict(cur_row: u16, _: u16, prev_col: u16, _: u16) -> u16 {
        cur_row.wrapping_add(prev_col)
    }
}

struct Up;
impl Predictor for Up {
    fn predict(cur_row: u16, prev_row: u16, _: u16, _: u16) -> u16 {
        cur_row.wrapping_add(prev_row)
    }
}

struct Avg;
impl Predictor for Avg {
    fn predict(cur_row: u16, prev_row: u16, prev_col: u16, _: u16) -> u16 {
        cur_row.wrapping_add(((prev_col as u32 + prev_row as u32) / 2) as u16)
    }
}

struct Paeth;
impl Predictor for Paeth {
    fn predict(cur_row: u16, prev_row: u16, prev_col: u16, top_left: u16) -> u16 {
        fn paeth(a: u16, b: u16, c: u16) -> u16 {
            let a = a as i32;
            let b = b as i32;
            let c = c as i32;

            let p = a + b - c;
            let pa = (p - a).abs();
            let pb = (p - b).abs();
            let pc = (p - c).abs();

            if pa <= pb && pa <= pc {
                a as u16
            } else if pb <= pc {
                b as u16
            } else {
                c as u16
            }
        }

        cur_row.wrapping_add(paeth(prev_col, prev_row, top_left))
    }
}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use crate::filter::lzw_flate::{PredictorParams, apply_predictor, flate, lzw};
    use crate::object::dict::Dict;

    #[test]
    fn decode_lzw() {
        let input = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let decoded = lzw::decode(&input, Dict::default()).unwrap();

        assert_eq!(decoded, vec![45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
    }

    #[test]
    fn decode_flate_zlib() {
        let input = [
            0x78, 0x9c, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x7, 0x0, 0x5, 0x8c, 0x1, 0xf5,
        ];

        let decoded = flate::decode(&input, Dict::default()).unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn decode_flate() {
        let input = [0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x7, 0x0];

        let decoded = flate::decode(&input, Dict::default()).unwrap();
        assert_eq!(decoded, b"Hello");
    }
    
    fn predictor_expected() -> Vec<u8> {
        vec![
            // Row 1
            127, 127, 127, 125, 129, 127, 123, 130, 128, 
            // Row 2
            128, 129, 126, 126, 132, 124, 121, 127, 126, 
            // Row 3
            131, 130, 122, 133, 129, 128, 127, 100, 126,
        ]
    }

    fn predictor_test(predictor: u8, input: &[u8]) {
        let params = PredictorParams {
            predictor,
            colors: 3,
            bits_per_component: 8,
            columns: 3,
            early_change: false,
        };

        let expected = predictor_expected();
        let out = apply_predictor(input.to_vec(), &params).unwrap();

        assert_eq!(expected, out);
    }

    #[test]
    fn predictor_sub() {
        predictor_test(
            11,
            &[
                // Row 1
                1, 127, 127, 127, 254, 2, 0, 254, 1, 1, 
                // Row 2
                1, 128, 129, 126, 254, 3, 254, 251, 251, 2, 
                // Row 3
                1, 131, 130, 122, 2, 255, 6, 250, 227, 254,
            ],
        );
    }

    #[test]
    fn predictor_up() {
        predictor_test(
            12,
            &[
                // Row 1
                2, 127, 127, 127, 125, 129, 127, 123, 130, 128, 
                // Row 2
                2, 1, 2, 255, 1, 3, 253, 254, 253, 254, 
                // Row 3
                2, 3, 1, 252, 7, 253, 4, 6, 229, 0,
            ],
        );
    }

    #[test]
    fn predictor_avg() {
        predictor_test(
            13,
            &[
                // Row 1
                3, 127, 127, 127, 62, 66, 64, 61, 66, 65, 
                // Row 2
                3, 65, 66, 63, 0, 3, 254, 253, 252, 0, 
                // Row 3
                3, 67, 66, 59, 5, 254, 5, 0, 228, 255,
            ],
        );
    }

    #[test]
    fn predictor_paeth() {
        predictor_test(
            14,
            &[
                // Row 1
                4, 127, 127, 127, 254, 2, 0, 254, 1, 1, 
                // Row 2
                4, 1, 2, 255, 1, 3, 254, 254, 251, 2, 
                // Row 3
                4, 3, 1, 252, 5, 253, 6, 1, 229, 254,
            ],
        );
    }
}
