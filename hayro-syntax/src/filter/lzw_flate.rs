use crate::bit_reader::{BitChunk, BitChunks, BitReader, BitWriter, bit_mask};
use crate::filter::png::{self, BytesPerPixel, RowFilter};
use crate::object::Dict;
use crate::object::dict::keys::{BITS_PER_COMPONENT, COLORS, COLUMNS, EARLY_CHANGE, PREDICTOR};
use alloc::vec;
use alloc::vec::Vec;

pub(crate) mod flate {
    use crate::filter::lzw_flate::{PredictorParams, apply_predictor};
    use crate::object::Dict;
    use alloc::vec::Vec;

    pub(crate) fn decode(data: &[u8], params: &Dict<'_>) -> Option<Vec<u8>> {
        let data = zlib_body(data).unwrap_or(data);
        // Broken encoders can refer to data before the start, which should read as zeros.
        let decoded = inflate(data, false)
            .or_else(|_| inflate(data, true))
            .unwrap_or_else(|partial| partial);
        let params = PredictorParams::from_params(params);
        apply_predictor(decoded, &params)
    }

    const WINDOW_SIZE: usize = 32 * 1024;

    fn zlib_body(data: &[u8]) -> Option<&[u8]> {
        let [cmf, flg, body @ ..] = data else {
            return None;
        };

        ((cmf & 0x0f) == 0x08
            && (u16::from(*cmf) << 8 | u16::from(*flg)).is_multiple_of(31)
            && (flg & 0x20) == 0)
            .then_some(body)
    }

    #[cfg(feature = "unsafe")]
    fn inflate(mut data: &[u8], zero_window: bool) -> Result<Vec<u8>, Vec<u8>> {
        use zlib_rs::{Inflate, InflateFlush, Status};

        static ZEROS: [u8; WINDOW_SIZE] = [0; WINDOW_SIZE];

        let mut inflate = Inflate::new(false, 15);
        if zero_window {
            inflate.set_dictionary(&ZEROS).ok();
        }
        let mut out = Vec::new();
        let mut written = 0;

        loop {
            out.resize((out.len() * 2).max(WINDOW_SIZE), 0);

            let (total_in, total_out) = (inflate.total_in(), inflate.total_out());
            let result = inflate.decompress(data, &mut out[written..], InflateFlush::Finish);
            let consumed = (inflate.total_in() - total_in) as usize;
            let produced = (inflate.total_out() - total_out) as usize;
            written += produced;
            data = data.get(consumed..).unwrap_or_default();

            match result {
                Ok(Status::Ok | Status::BufError) if consumed != 0 || produced != 0 => {}
                Ok(_) => {
                    out.truncate(written);
                    return Ok(out);
                }
                Err(_) => {
                    out.truncate(written);
                    return Err(out);
                }
            }
        }
    }

    #[cfg(not(feature = "unsafe"))]
    fn inflate(mut data: &[u8], zero_window: bool) -> Result<Vec<u8>, Vec<u8>> {
        use alloc::boxed::Box;
        use alloc::vec;
        use miniz_oxide::inflate::TINFLStatus;
        use miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
        use miniz_oxide::inflate::core::{DecompressorOxide, decompress};

        let start = if zero_window { WINDOW_SIZE } else { 0 };
        let mut decompressor = Box::<DecompressorOxide>::default();
        let mut out = vec![0; start];
        let mut written = start;

        loop {
            out.resize((out.len() * 2).max(WINDOW_SIZE), 0);

            let (status, consumed, produced) = decompress(
                &mut decompressor,
                data,
                &mut out,
                written,
                TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
            );
            written += produced;
            data = data.get(consumed..).unwrap_or_default();

            if status != TINFLStatus::HasMoreOutput {
                out.truncate(written);
                out.drain(..start);

                return match status {
                    TINFLStatus::Done | TINFLStatus::FailedCannotMakeProgress => Ok(out),
                    _ => Err(out),
                };
            }
        }
    }
}

pub(crate) mod lzw {
    use crate::bit_reader::BitReader;
    use crate::filter::lzw_flate::{PredictorParams, apply_predictor};
    use crate::object::Dict;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Decode a LZW-encoded stream.
    pub(crate) fn decode(data: &[u8], params: &Dict<'_>) -> Option<Vec<u8>> {
        let params = PredictorParams::from_params(params);

        let decoded = decode_impl(data, params.early_change)?;

        apply_predictor(decoded, &params)
    }

    const CLEAR_TABLE: usize = 256;
    const EOD: usize = 257;
    const MAX_ENTRIES: usize = 4096;
    const INITIAL_SIZE: u16 = 258;

    fn decode_impl(data: &[u8], early_change: bool) -> Option<Vec<u8>> {
        let mut table = Table::new(early_change);
        let mut bit_size = table.code_length();
        let mut reader = BitReader::new(data);
        let mut decoded = vec![];
        let mut prev = None;

        loop {
            let next = match reader.read(bit_size) {
                Some(code) => code as usize,
                None => {
                    return Some(decoded);
                }
            };

            match next {
                CLEAR_TABLE => {
                    table.clear();
                    prev = None;
                    bit_size = table.code_length();
                }
                EOD => return Some(decoded),
                new => {
                    if new > table.size() {
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
                        return None;
                    }

                    bit_size = table.code_length();
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

    fn row_length_in_bytes(&self) -> usize {
        (self.columns * self.bits_per_pixel() as usize).div_ceil(8)
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
    fn from_params(dict: &Dict<'_>) -> Self {
        Self {
            predictor: dict.get(PREDICTOR).unwrap_or(1),
            colors: dict.get(COLORS).unwrap_or(1),
            bits_per_component: dict.get(BITS_PER_COMPONENT).unwrap_or(8),
            columns: dict.get(COLUMNS).unwrap_or(1),
            early_change: dict.get::<u8>(EARLY_CHANGE).map(|e| e != 0).unwrap_or(true),
        }
    }
}

fn apply_predictor(data: Vec<u8>, params: &PredictorParams) -> Option<Vec<u8>> {
    match params.predictor {
        1 => Some(data),
        i => {
            let is_png_predictor = i >= 10;

            let row_len = params.row_length_in_bytes();

            let total_row_len = if is_png_predictor {
                // + 1 Because each row must start with the predictor that is used for PNG predictors.
                row_len + 1
            } else {
                row_len
            };

            let num_rows = data.len() / total_row_len;

            if !matches!(params.bits_per_component, 1 | 2 | 4 | 8 | 16) {
                return None;
            }

            let (bit_size, chunk_len) = if is_png_predictor {
                (
                    8,
                    (params.colors * params.bits_per_component).div_ceil(8) as usize,
                )
            } else {
                (params.bits_per_component, params.colors as usize)
            };

            if bit_size == 8
                && let Some(tbpp) = BytesPerPixel::from_row_len(row_len, chunk_len)
            {
                return apply_predictor_8bit_png(
                    data,
                    i,
                    is_png_predictor,
                    row_len,
                    total_row_len,
                    num_rows,
                    tbpp,
                );
            }

            let zero_row = vec![0; row_len];
            let mut prev_row = BitChunks::new(&zero_row, bit_size, chunk_len)?;
            let zero_col = BitChunk::new(0, chunk_len);
            let mut out = vec![0; num_rows * row_len];
            let mut writer = BitWriter::new(&mut out, bit_size)?;

            for in_row in data.chunks_exact(total_row_len) {
                if is_png_predictor {
                    let predictor = in_row[0];
                    let in_data = &in_row[1..];
                    let in_data_chunks = BitChunks::new(in_data, bit_size, chunk_len)?;

                    match predictor {
                        1 => apply::<Sub>(
                            prev_row,
                            zero_col.clone(),
                            zero_col.clone(),
                            in_data_chunks,
                            &mut writer,
                            chunk_len,
                            bit_size,
                        )?,
                        2 => apply::<Up>(
                            prev_row,
                            zero_col.clone(),
                            zero_col.clone(),
                            in_data_chunks,
                            &mut writer,
                            chunk_len,
                            bit_size,
                        )?,
                        3 => apply::<Avg>(
                            prev_row,
                            zero_col.clone(),
                            zero_col.clone(),
                            in_data_chunks,
                            &mut writer,
                            chunk_len,
                            bit_size,
                        )?,
                        4 => apply::<Paeth>(
                            prev_row,
                            zero_col.clone(),
                            zero_col.clone(),
                            in_data_chunks,
                            &mut writer,
                            chunk_len,
                            bit_size,
                        )?,
                        _ => {
                            // Just copy the data.
                            let mut reader = BitReader::new(in_data);

                            while let Some(data) = reader.read(bit_size) {
                                writer.write(data);
                            }
                        }
                    }
                } else if i == 2 {
                    apply::<Sub>(
                        prev_row,
                        zero_col.clone(),
                        zero_col.clone(),
                        BitChunks::new(in_row, bit_size, chunk_len)?,
                        &mut writer,
                        chunk_len,
                        bit_size,
                    );
                } else {
                    return None;
                }

                let (data, new_writer) = writer.split_off();
                writer = new_writer;
                prev_row = BitChunks::new(data, bit_size, chunk_len)?;
            }

            Some(out)
        }
    }
}

fn apply_predictor_8bit_png(
    mut data: Vec<u8>,
    predictor: u8,
    is_png_predictor: bool,
    row_len: usize,
    total_row_len: usize,
    num_rows: usize,
    tbpp: BytesPerPixel,
) -> Option<Vec<u8>> {
    if row_len == 0 || total_row_len == 0 {
        return None;
    }

    let out_len = num_rows * row_len;

    for row_idx in 0..num_rows {
        let src_start = row_idx * total_row_len;
        let dst_start = row_idx * row_len;

        if is_png_predictor {
            let predictor = data[src_start];
            data.copy_within(src_start + 1..src_start + 1 + row_len, dst_start);

            let (done, rest) = data.split_at_mut(dst_start);
            let prev_row = if let Some(prev_start) = done.len().checked_sub(row_len) {
                &done[prev_start..]
            } else {
                &[]
            };
            let row = &mut rest[..row_len];

            if let Some(filter) = RowFilter::from_u8(predictor) {
                png::unfilter(filter, tbpp, prev_row, row);
            }
        } else if predictor == 2 {
            let (done, rest) = data.split_at_mut(dst_start);
            let prev_row = if let Some(prev_start) = done.len().checked_sub(row_len) {
                &done[prev_start..]
            } else {
                &[]
            };
            let row = &mut rest[..row_len];
            png::unfilter(RowFilter::Sub, tbpp, prev_row, row);
        } else {
            return None;
        }
    }

    data.truncate(out_len);

    Some(data)
}

fn apply<'a, T: Predictor>(
    prev_row: BitChunks<'a>,
    mut prev_col: BitChunk,
    mut top_left: BitChunk,
    cur_row: BitChunks<'a>,
    writer: &mut BitWriter<'a>,
    chunk_len: usize,
    bit_size: u8,
) -> Option<()> {
    for (cur_row, prev_row) in cur_row.zip(prev_row) {
        let old_pos = writer.cur_pos();

        for (((cur_row, prev_row), prev_col), top_left) in cur_row
            .iter()
            .zip(prev_row.iter())
            .zip(prev_col.iter())
            .zip(top_left.iter())
        {
            // Note that the wrapping behavior when adding inside the predictors is dependent on the
            // bit size, so it wouldn't be triggered for bits per component < 16. So we mask out
            // the bytes manually, which is equivalent to a wrapping add.
            writer.write(
                T::predict(cur_row, prev_row, prev_col, top_left) as u32 & bit_mask(bit_size),
            );
        }

        prev_col = {
            let out_data = writer.get_data();
            let mut reader = BitReader::new_with(out_data, old_pos);
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
    use crate::object::Dict;

    #[test]
    fn decode_lzw() {
        let input = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let decoded = lzw::decode(&input, &Dict::default()).unwrap();

        assert_eq!(decoded, vec![45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
    }

    #[test]
    fn decode_flate_zlib() {
        let input = [
            0x78, 0x9c, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x7, 0x0, 0x5, 0x8c, 0x1, 0xf5,
        ];

        let decoded = flate::decode(&input, &Dict::default()).unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn decode_flate() {
        let input = [0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x7, 0x0];

        let decoded = flate::decode(&input, &Dict::default()).unwrap();
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
