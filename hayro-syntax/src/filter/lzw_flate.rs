use crate::object::Dict;
use crate::object::dict::keys::{BITS_PER_COMPONENT, COLORS, COLUMNS, EARLY_CHANGE, PREDICTOR};
use hayro_common::bit::{BitChunk, BitChunks, BitReader, BitWriter, bit_mask};
use log::warn;

pub(crate) mod flate {
    use crate::filter::lzw_flate::{PredictorParams, apply_predictor};
    use crate::object::Dict;
    use flate2::read::{DeflateDecoder, ZlibDecoder};
    use log::warn;
    use std::io::Read;

    pub(crate) fn decode(data: &[u8], params: Dict) -> Option<Vec<u8>> {
        let decoded = zlib_stream(data)
            .or_else(|| deflate_stream(data))
            .or_else(|| {
                warn!("flate stream is broken, decoding with fallback");

                fallback::decode(data)
            })?;
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

    /// Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/flate_stream.js>
    /// TODO: Rewrite this in idiomatic Rust.
    mod fallback {
        use log::warn;

        pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
            flate_decode(data)
        }

        fn flate_decode(data: &[u8]) -> Option<Vec<u8>> {
            if data.len() >= 2 {
                let cmf = data[0];
                let flg = data[1];

                if (cmf & 0x0f) == 0x08
                    && ((cmf as u16) << 8 | flg as u16).is_multiple_of(31)
                    && (flg & 0x20) == 0
                {
                    let mut stream = FlateStream::new(&data[2..]);
                    return stream.decode();
                }
            }

            let mut stream = FlateStream::new(data);
            stream.decode()
        }

        struct FlateStream<'a> {
            data: &'a [u8],
            pos: usize,
            code_buf: u32,
            code_size: u8,
            output: Vec<u8>,
            eof: bool,
        }

        impl<'a> FlateStream<'a> {
            fn new(data: &'a [u8]) -> Self {
                FlateStream {
                    data,
                    pos: 0,
                    code_buf: 0,
                    code_size: 0,
                    output: Vec::new(),
                    eof: false,
                }
            }

            fn decode(&mut self) -> Option<Vec<u8>> {
                while !self.eof && self.pos < self.data.len() {
                    self.read_block();
                }

                Some(std::mem::take(&mut self.output))
            }

            fn get_byte(&mut self) -> Option<u8> {
                if self.pos >= self.data.len() {
                    None
                } else {
                    let byte = self.data[self.pos];
                    self.pos += 1;
                    Some(byte)
                }
            }

            fn peek_byte(&self) -> Option<u8> {
                if self.pos >= self.data.len() {
                    None
                } else {
                    Some(self.data[self.pos])
                }
            }

            fn get_bytes(&mut self, n: usize) -> Vec<u8> {
                let end = (self.pos + n).min(self.data.len());
                let bytes = self.data[self.pos..end].to_vec();
                self.pos = end;
                bytes
            }

            fn get_bits(&mut self, bits: u8) -> Option<u32> {
                while self.code_size < bits {
                    let b = self.get_byte()?;
                    self.code_buf |= (b as u32) << self.code_size;
                    self.code_size += 8;
                }

                let result = self.code_buf & ((1 << bits) - 1);
                self.code_buf >>= bits;
                self.code_size -= bits;

                Some(result)
            }

            fn get_code(&mut self, table: &HuffmanTable) -> Option<u16> {
                let codes = &table.codes;
                let max_len = table.max_len;

                while self.code_size < max_len {
                    if let Some(b) = self.get_byte() {
                        self.code_buf |= (b as u32) << self.code_size;
                        self.code_size += 8;
                    } else {
                        // Premature end of stream
                        break;
                    }
                }

                let code = codes.get((self.code_buf & ((1 << max_len) - 1)) as usize)?;
                let code_len = (code >> 16) as u8;
                let code_val = code & 0xffff;

                if code_len < 1 || self.code_size < code_len {
                    return None;
                }

                self.code_buf >>= code_len;
                self.code_size -= code_len;

                Some(code_val as u16)
            }

            fn read_block(&mut self) {
                // Read block header
                let hdr = match self.get_bits(3) {
                    Some(h) => h,
                    None => {
                        warn!("bad block header in flate stream");
                        self.eof = true;
                        return;
                    }
                };

                if (hdr & 1) != 0 {
                    self.eof = true;
                }

                let hdr = hdr >> 1;

                match hdr {
                    0 => self.read_uncompressed_block(),
                    1 => self.read_compressed_block(true),
                    2 => self.read_compressed_block(false),
                    _ => {
                        warn!("unknown block type in flate stream");
                        self.eof = true;
                    }
                }
            }

            fn read_uncompressed_block(&mut self) {
                // Skip any remaining bits in current byte
                self.code_buf = 0;
                self.code_size = 0;

                let len_low = match self.get_byte() {
                    Some(b) => b as u16,
                    None => {
                        warn!("bad block header in flate stream");
                        self.eof = true;
                        return;
                    }
                };

                let len_high = match self.get_byte() {
                    Some(b) => b as u16,
                    None => {
                        warn!("bad block header in flate stream");
                        self.eof = true;
                        return;
                    }
                };

                let block_len = len_low | (len_high << 8);

                let nlen_low = match self.get_byte() {
                    Some(b) => b as u16,
                    None => {
                        warn!("bad block header in flate stream");
                        self.eof = true;
                        return;
                    }
                };

                let nlen_high = match self.get_byte() {
                    Some(b) => b as u16,
                    None => {
                        warn!("bad block header in flate stream");
                        self.eof = true;
                        return;
                    }
                };

                let check = nlen_low | (nlen_high << 8);

                if check != !block_len && (block_len != 0 || check != 0) {
                    // Ignoring error for bad "empty" block
                    warn!("bad uncompressed block length in flate stream");
                }

                if block_len == 0 {
                    if self.peek_byte().is_none() {
                        self.eof = true;
                    }
                } else {
                    let block = self.get_bytes(block_len as usize);
                    self.output.extend_from_slice(&block);
                    if block.len() < block_len as usize {
                        self.eof = true;
                    }
                }
            }

            fn read_compressed_block(&mut self, fixed: bool) {
                let (lit_code_table, dist_code_table) = if fixed {
                    (get_fixed_lit_table(), get_fixed_dist_table())
                } else {
                    match self.read_dynamic_tables() {
                        Some(tables) => tables,
                        None => {
                            self.eof = true;
                            return;
                        }
                    }
                };

                loop {
                    let code1 = match self.get_code(&lit_code_table) {
                        Some(c) => c,
                        None => {
                            self.eof = true;
                            return;
                        }
                    };

                    if code1 < 256 {
                        self.output.push(code1 as u8);
                    } else if code1 == 256 {
                        return;
                    } else {
                        let code1 = code1 - 257;
                        let length_info = LENGTH_DECODE.get(code1 as usize).copied().unwrap_or(0);
                        let extra_bits = (length_info >> 16) as u8;
                        let mut length = (length_info & 0xffff) as usize;

                        if extra_bits > 0 {
                            if let Some(extra) = self.get_bits(extra_bits) {
                                length += extra as usize;
                            } else {
                                self.eof = true;
                                return;
                            }
                        }

                        let dist_code = match self.get_code(&dist_code_table) {
                            Some(c) => c,
                            None => {
                                self.eof = true;
                                return;
                            }
                        };

                        let dist_info = DIST_DECODE[dist_code as usize];
                        let extra_bits = (dist_info >> 16) as u8;
                        let mut distance = (dist_info & 0xffff) as usize;

                        if extra_bits > 0 {
                            if let Some(extra) = self.get_bits(extra_bits) {
                                distance += extra as usize;
                            } else {
                                self.eof = true;
                                return;
                            }
                        }

                        // Copy from previous output
                        let start = self.output.len().wrapping_sub(distance);
                        for _ in 0..length {
                            if start < self.output.len() {
                                let byte = self.output[self.output.len() - distance];
                                self.output.push(byte);
                            }
                        }
                    }
                }
            }

            fn read_dynamic_tables(&mut self) -> Option<(HuffmanTable, HuffmanTable)> {
                let num_lit_codes = self.get_bits(5)? as usize + 257;
                let num_dist_codes = self.get_bits(5)? as usize + 1;
                let num_code_len_codes = self.get_bits(4)? as usize + 4;

                // Build code length code table
                let mut code_len_code_lengths = vec![0u8; 19];
                for i in 0..num_code_len_codes {
                    code_len_code_lengths[CODE_LEN_CODE_MAP[i] as usize] = self.get_bits(3)? as u8;
                }

                let code_len_table = generate_huffman_table(&code_len_code_lengths);

                // Read code lengths
                let total_codes = num_lit_codes + num_dist_codes;
                let mut code_lengths = vec![0u8; total_codes];
                let mut i = 0;

                while i < total_codes {
                    let code = self.get_code(&code_len_table)?;

                    match code {
                        0..=15 => {
                            code_lengths[i] = code as u8;
                            i += 1;
                        }
                        16 => {
                            // Repeat previous
                            let repeat_count = self.get_bits(2)? as usize + 3;
                            let prev = if i > 0 { code_lengths[i - 1] } else { 0 };
                            for _ in 0..repeat_count {
                                if i < total_codes {
                                    code_lengths[i] = prev;
                                    i += 1;
                                }
                            }
                        }
                        17 => {
                            // Repeat zero 3-10 times
                            let repeat_count = self.get_bits(3)? as usize + 3;
                            for _ in 0..repeat_count {
                                if i < total_codes {
                                    code_lengths[i] = 0;
                                    i += 1;
                                }
                            }
                        }
                        18 => {
                            // Repeat zero 11-138 times
                            let repeat_count = self.get_bits(7)? as usize + 11;
                            for _ in 0..repeat_count {
                                if i < total_codes {
                                    code_lengths[i] = 0;
                                    i += 1;
                                }
                            }
                        }
                        _ => return None,
                    }
                }

                let lit_table = generate_huffman_table(&code_lengths[..num_lit_codes]);
                let dist_table = generate_huffman_table(&code_lengths[num_lit_codes..]);

                Some((lit_table, dist_table))
            }
        }

        struct HuffmanTable {
            codes: Vec<u32>,
            max_len: u8,
        }

        fn generate_huffman_table(lengths: &[u8]) -> HuffmanTable {
            let _n = lengths.len();

            // Find max code length
            let max_len = lengths.iter().cloned().max().unwrap_or(0);

            if max_len == 0 {
                return HuffmanTable {
                    codes: vec![0; 1],
                    max_len: 1,
                };
            }

            // Build the table
            let size = 1 << max_len;
            let mut codes = vec![0u32; size];

            let mut code = 0u32;
            for len in 1..=max_len {
                for (val, &length) in lengths.iter().enumerate() {
                    if length == len {
                        // Bit-reverse the code
                        let mut code2 = 0u32;
                        let mut t = code;
                        for _ in 0..len {
                            code2 = (code2 << 1) | (t & 1);
                            t >>= 1;
                        }

                        // Fill the table entries
                        let skip = 1 << len;
                        let mut i = code2 as usize;
                        while i < size {
                            codes[i] = ((len as u32) << 16) | (val as u32);
                            i += skip;
                        }
                        code += 1;
                    }
                }
                code <<= 1;
            }

            HuffmanTable { codes, max_len }
        }

        fn get_fixed_lit_table() -> HuffmanTable {
            HuffmanTable {
                codes: FIXED_LIT_CODE_TAB.to_vec(),
                max_len: 9,
            }
        }

        fn get_fixed_dist_table() -> HuffmanTable {
            HuffmanTable {
                codes: FIXED_DIST_CODE_TAB.to_vec(),
                max_len: 5,
            }
        }

        const CODE_LEN_CODE_MAP: [u8; 19] = [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];

        const LENGTH_DECODE: [u32; 29] = [
            0x00003, 0x00004, 0x00005, 0x00006, 0x00007, 0x00008, 0x00009, 0x0000a, 0x1000b,
            0x1000d, 0x1000f, 0x10011, 0x20013, 0x20017, 0x2001b, 0x2001f, 0x30023, 0x3002b,
            0x30033, 0x3003b, 0x40043, 0x40053, 0x40063, 0x40073, 0x50083, 0x500a3, 0x500c3,
            0x500e3, 0x00102,
        ];

        const DIST_DECODE: [u32; 30] = [
            0x00001, 0x00002, 0x00003, 0x00004, 0x10005, 0x10007, 0x20009, 0x2000d, 0x30011,
            0x30019, 0x40021, 0x40031, 0x50041, 0x50061, 0x60081, 0x600c1, 0x70101, 0x70181,
            0x80201, 0x80301, 0x90401, 0x90601, 0xa0801, 0xa0c01, 0xb1001, 0xb1801, 0xc2001,
            0xc3001, 0xd4001, 0xd6001,
        ];

        const FIXED_LIT_CODE_TAB: [u32; 512] = [
            0x70100, 0x80050, 0x80010, 0x80118, 0x70110, 0x80070, 0x80030, 0x900c0, 0x70108,
            0x80060, 0x80020, 0x900a0, 0x80000, 0x80080, 0x80040, 0x900e0, 0x70104, 0x80058,
            0x80018, 0x90090, 0x70114, 0x80078, 0x80038, 0x900d0, 0x7010c, 0x80068, 0x80028,
            0x900b0, 0x80008, 0x80088, 0x80048, 0x900f0, 0x70102, 0x80054, 0x80014, 0x8011c,
            0x70112, 0x80074, 0x80034, 0x900c8, 0x7010a, 0x80064, 0x80024, 0x900a8, 0x80004,
            0x80084, 0x80044, 0x900e8, 0x70106, 0x8005c, 0x8001c, 0x90098, 0x70116, 0x8007c,
            0x8003c, 0x900d8, 0x7010e, 0x8006c, 0x8002c, 0x900b8, 0x8000c, 0x8008c, 0x8004c,
            0x900f8, 0x70101, 0x80052, 0x80012, 0x8011a, 0x70111, 0x80072, 0x80032, 0x900c4,
            0x70109, 0x80062, 0x80022, 0x900a4, 0x80002, 0x80082, 0x80042, 0x900e4, 0x70105,
            0x8005a, 0x8001a, 0x90094, 0x70115, 0x8007a, 0x8003a, 0x900d4, 0x7010d, 0x8006a,
            0x8002a, 0x900b4, 0x8000a, 0x8008a, 0x8004a, 0x900f4, 0x70103, 0x80056, 0x80016,
            0x8011e, 0x70113, 0x80076, 0x80036, 0x900cc, 0x7010b, 0x80066, 0x80026, 0x900ac,
            0x80006, 0x80086, 0x80046, 0x900ec, 0x70107, 0x8005e, 0x8001e, 0x9009c, 0x70117,
            0x8007e, 0x8003e, 0x900dc, 0x7010f, 0x8006e, 0x8002e, 0x900bc, 0x8000e, 0x8008e,
            0x8004e, 0x900fc, 0x70100, 0x80051, 0x80011, 0x80119, 0x70110, 0x80071, 0x80031,
            0x900c2, 0x70108, 0x80061, 0x80021, 0x900a2, 0x80001, 0x80081, 0x80041, 0x900e2,
            0x70104, 0x80059, 0x80019, 0x90092, 0x70114, 0x80079, 0x80039, 0x900d2, 0x7010c,
            0x80069, 0x80029, 0x900b2, 0x80009, 0x80089, 0x80049, 0x900f2, 0x70102, 0x80055,
            0x80015, 0x8011d, 0x70112, 0x80075, 0x80035, 0x900ca, 0x7010a, 0x80065, 0x80025,
            0x900aa, 0x80005, 0x80085, 0x80045, 0x900ea, 0x70106, 0x8005d, 0x8001d, 0x9009a,
            0x70116, 0x8007d, 0x8003d, 0x900da, 0x7010e, 0x8006d, 0x8002d, 0x900ba, 0x8000d,
            0x8008d, 0x8004d, 0x900fa, 0x70101, 0x80053, 0x80013, 0x8011b, 0x70111, 0x80073,
            0x80033, 0x900c6, 0x70109, 0x80063, 0x80023, 0x900a6, 0x80003, 0x80083, 0x80043,
            0x900e6, 0x70105, 0x8005b, 0x8001b, 0x90096, 0x70115, 0x8007b, 0x8003b, 0x900d6,
            0x7010d, 0x8006b, 0x8002b, 0x900b6, 0x8000b, 0x8008b, 0x8004b, 0x900f6, 0x70103,
            0x80057, 0x80017, 0x8011f, 0x70113, 0x80077, 0x80037, 0x900ce, 0x7010b, 0x80067,
            0x80027, 0x900ae, 0x80007, 0x80087, 0x80047, 0x900ee, 0x70107, 0x8005f, 0x8001f,
            0x9009e, 0x70117, 0x8007f, 0x8003f, 0x900de, 0x7010f, 0x8006f, 0x8002f, 0x900be,
            0x8000f, 0x8008f, 0x8004f, 0x900fe, 0x70100, 0x80050, 0x80010, 0x80118, 0x70110,
            0x80070, 0x80030, 0x900c1, 0x70108, 0x80060, 0x80020, 0x900a1, 0x80000, 0x80080,
            0x80040, 0x900e1, 0x70104, 0x80058, 0x80018, 0x90091, 0x70114, 0x80078, 0x80038,
            0x900d1, 0x7010c, 0x80068, 0x80028, 0x900b1, 0x80008, 0x80088, 0x80048, 0x900f1,
            0x70102, 0x80054, 0x80014, 0x8011c, 0x70112, 0x80074, 0x80034, 0x900c9, 0x7010a,
            0x80064, 0x80024, 0x900a9, 0x80004, 0x80084, 0x80044, 0x900e9, 0x70106, 0x8005c,
            0x8001c, 0x90099, 0x70116, 0x8007c, 0x8003c, 0x900d9, 0x7010e, 0x8006c, 0x8002c,
            0x900b9, 0x8000c, 0x8008c, 0x8004c, 0x900f9, 0x70101, 0x80052, 0x80012, 0x8011a,
            0x70111, 0x80072, 0x80032, 0x900c5, 0x70109, 0x80062, 0x80022, 0x900a5, 0x80002,
            0x80082, 0x80042, 0x900e5, 0x70105, 0x8005a, 0x8001a, 0x90095, 0x70115, 0x8007a,
            0x8003a, 0x900d5, 0x7010d, 0x8006a, 0x8002a, 0x900b5, 0x8000a, 0x8008a, 0x8004a,
            0x900f5, 0x70103, 0x80056, 0x80016, 0x8011e, 0x70113, 0x80076, 0x80036, 0x900cd,
            0x7010b, 0x80066, 0x80026, 0x900ad, 0x80006, 0x80086, 0x80046, 0x900ed, 0x70107,
            0x8005e, 0x8001e, 0x9009d, 0x70117, 0x8007e, 0x8003e, 0x900dd, 0x7010f, 0x8006e,
            0x8002e, 0x900bd, 0x8000e, 0x8008e, 0x8004e, 0x900fd, 0x70100, 0x80051, 0x80011,
            0x80119, 0x70110, 0x80071, 0x80031, 0x900c3, 0x70108, 0x80061, 0x80021, 0x900a3,
            0x80001, 0x80081, 0x80041, 0x900e3, 0x70104, 0x80059, 0x80019, 0x90093, 0x70114,
            0x80079, 0x80039, 0x900d3, 0x7010c, 0x80069, 0x80029, 0x900b3, 0x80009, 0x80089,
            0x80049, 0x900f3, 0x70102, 0x80055, 0x80015, 0x8011d, 0x70112, 0x80075, 0x80035,
            0x900cb, 0x7010a, 0x80065, 0x80025, 0x900ab, 0x80005, 0x80085, 0x80045, 0x900eb,
            0x70106, 0x8005d, 0x8001d, 0x9009b, 0x70116, 0x8007d, 0x8003d, 0x900db, 0x7010e,
            0x8006d, 0x8002d, 0x900bb, 0x8000d, 0x8008d, 0x8004d, 0x900fb, 0x70101, 0x80053,
            0x80013, 0x8011b, 0x70111, 0x80073, 0x80033, 0x900c7, 0x70109, 0x80063, 0x80023,
            0x900a7, 0x80003, 0x80083, 0x80043, 0x900e7, 0x70105, 0x8005b, 0x8001b, 0x90097,
            0x70115, 0x8007b, 0x8003b, 0x900d7, 0x7010d, 0x8006b, 0x8002b, 0x900b7, 0x8000b,
            0x8008b, 0x8004b, 0x900f7, 0x70103, 0x80057, 0x80017, 0x8011f, 0x70113, 0x80077,
            0x80037, 0x900cf, 0x7010b, 0x80067, 0x80027, 0x900af, 0x80007, 0x80087, 0x80047,
            0x900ef, 0x70107, 0x8005f, 0x8001f, 0x9009f, 0x70117, 0x8007f, 0x8003f, 0x900df,
            0x7010f, 0x8006f, 0x8002f, 0x900bf, 0x8000f, 0x8008f, 0x8004f, 0x900ff,
        ];

        const FIXED_DIST_CODE_TAB: [u32; 32] = [
            0x50000, 0x50010, 0x50008, 0x50018, 0x50004, 0x50014, 0x5000c, 0x5001c, 0x50002,
            0x50012, 0x5000a, 0x5001a, 0x50006, 0x50016, 0x5000e, 0x00000, 0x50001, 0x50011,
            0x50009, 0x50019, 0x50005, 0x50015, 0x5000d, 0x5001d, 0x50003, 0x50013, 0x5000b,
            0x5001b, 0x50007, 0x50017, 0x5000f, 0x00000,
        ];
    }
}

pub(crate) mod lzw {
    use crate::filter::lzw_flate::{PredictorParams, apply_predictor};
    use crate::object::Dict;
    use hayro_common::bit::BitReader;
    use log::warn;

    /// Decode a LZW-encoded stream.
    pub(crate) fn decode(data: &[u8], params: Dict) -> Option<Vec<u8>> {
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
        let mut bit_size = table.code_length();
        let mut reader = BitReader::new(data);
        let mut decoded = vec![];
        let mut prev = None;

        loop {
            let next = match reader.read(bit_size) {
                Some(code) => code as usize,
                None => {
                    warn!("premature EOF in LZW stream, EOD code missing");
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
                        warn!("invalid LZW code: {} (table size: {})", new, table.size());
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
                        warn!("LZW decode error: code {new} not found and prev is None");
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
    fn from_params(dict: &Dict) -> Self {
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
                warn!("invalid bits per component {}", params.bits_per_component);

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
                    warn!("unknown predictor {i}");

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
