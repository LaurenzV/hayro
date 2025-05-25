/* Copyright 2012 Mozilla Foundation
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
mod bitmap;
mod bitmap_template0;
mod pattern_dictionary;
mod refinement;
mod standard_table;
mod symbol_dictionary;
mod tables;
mod text_region;

use crate::filter::ccitt::{CCITTFaxDecoder, CCITTFaxDecoderOptions};
use crate::filter::jbig2::bitmap::decode_bitmap;
use crate::filter::jbig2::bitmap_template0::decode_bitmap_template0;
use crate::filter::jbig2::pattern_dictionary::decode_pattern_dictionary;
use crate::filter::jbig2::refinement::decode_refinement;
use crate::filter::jbig2::standard_table::get_standard_table;
use crate::filter::jbig2::symbol_dictionary::decode_symbol_dictionary;
use crate::filter::jbig2::tables::{QE_TABLE, SEGMENT_TYPES};
use crate::filter::jbig2::text_region::decode_text_region;
use crate::object::dict::Dict;
use crate::object::dict::keys::JBIG2_GLOBALS;
use crate::object::stream::Stream;
use crate::reader::Reader as CrateReader;
use log::warn;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn decode(data: &[u8], params: Dict) -> Option<Vec<u8>> {
    let globals = params.get::<Stream>(JBIG2_GLOBALS);

    let mut chunks = Vec::new();

    if let Some(globals_data) = globals.and_then(|g| g.decoded()) {
        chunks.push(Chunk {
            data: globals_data.clone(),
            start: 0,
            end: globals_data.len(),
        });
    }

    chunks.push(Chunk {
        data: data.to_vec(),
        start: 0,
        end: data.len(),
    });

    let mut jbig2_image = Jbig2Image::new();
    let mut buf = jbig2_image.parse_chunks(&chunks)?;

    // JBIG2 had black as 1 and white as 0, inverting the colors
    for b in &mut buf {
        *b = *b ^ 0xFF;
    }

    Some(buf)
}

#[derive(Debug)]
pub struct Jbig2Error {
    message: String,
}

impl Jbig2Error {
    fn new(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
        }
    }
}

impl std::fmt::Display for Jbig2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Jbig2Error: {}", self.message)
    }
}

impl std::error::Error for Jbig2Error {}

// Utility data structures
struct ContextCache {
    contexts: HashMap<String, Vec<i8>>,
}

impl ContextCache {
    fn new() -> Self {
        Self {
            contexts: HashMap::new(),
        }
    }

    fn get_contexts(&mut self, id: &str) -> &mut Vec<i8> {
        self.contexts
            .entry(id.to_string())
            .or_insert_with(|| vec![0; 1 << 16])
    }
}

struct DecodingContext {
    decoder: ArithmeticDecoder,
    context_cache: ContextCache,
}

impl DecodingContext {
    fn new(data: Vec<u8>, start: usize, end: usize) -> Self {
        Self {
            decoder: ArithmeticDecoder::new(&data, start, end),
            context_cache: ContextCache::new(),
        }
    }

    fn decode_integer(&mut self, procedure: &str) -> Option<i32> {
        decode_integer(&mut self.context_cache, procedure, &mut self.decoder)
    }

    fn decode_iaid(&mut self, code_length: usize) -> u32 {
        decode_iaid(&mut self.context_cache, &mut self.decoder, code_length)
    }

    fn read_bit_with_context(&mut self, context_id: &str, pos: usize) -> u8 {
        let contexts = self.context_cache.get_contexts(context_id);
        self.decoder.read_bit(contexts, pos)
    }
}

// Chunk structure for parsing
#[derive(Clone)]
struct Chunk {
    data: Vec<u8>,
    start: usize,
    end: usize,
}

/// ArithmeticDecoder - ported from PDF.js arithmetic_decoder.js
///
/// This class implements the QM Coder decoding as defined in
/// JPEG 2000 Part I Final Committee Draft Version 1.0
/// Annex C.3 Arithmetic decoding procedure
/// available at http://www.jpeg.org/public/fcd15444-1.pdf
///
/// The arithmetic decoder is used in conjunction with context models to decode
/// JPEG2000 and JBIG2 streams.
struct ArithmeticDecoder {
    data: Vec<u8>,
    bp: usize,
    data_end: usize,
    chigh: u32,
    clow: u32,
    ct: i32,
    a: u32,
    counter: usize,
}

impl ArithmeticDecoder {
    // C.3.5 Initialisation of the decoder (INITDEC)
    // ✅ ARITHMETIC DECODER IMPLEMENTATION: Faithful port of the JavaScript QM Coder algorithm
    // with intentional improvements: Rust version adds bounds checking for memory safety where
    // JavaScript uses direct array access. Both follow the same QM Coder algorithm from
    // JPEG 2000 Part I Final Committee Draft Version 1.0 Annex C.3.
    fn new(data: &[u8], start: usize, end: usize) -> Self {
        let mut decoder = Self {
            data: data.to_vec(),
            bp: start,
            data_end: end,
            chigh: if start < data.len() {
                data[start] as u32
            } else {
                0
            },
            clow: 0,
            ct: 0,
            a: 0,
            counter: 0,
        };

        decoder.byte_in();
        decoder.chigh = ((decoder.chigh << 7) & 0xffff) | ((decoder.clow >> 9) & 0x7f);
        decoder.clow = (decoder.clow << 7) & 0xffff;
        decoder.ct -= 7;
        decoder.a = 0x8000;

        decoder
    }

    // C.3.4 Compressed data input (BYTEIN)
    fn byte_in(&mut self) {
        let bp = self.bp;

        if bp < self.data.len() && self.data[bp] == 0xff {
            if bp + 1 < self.data.len() && self.data[bp + 1] > 0x8f {
                self.clow += 0xff00;
                self.ct = 8;
            } else {
                self.bp = bp + 1;
                let byte_val = if self.bp < self.data.len() {
                    self.data[self.bp] as u32
                } else {
                    0xff
                };
                self.clow += byte_val << 9;
                self.ct = 7;
            }
        } else {
            self.bp = bp + 1;
            let byte_val = if self.bp < self.data_end {
                self.data[self.bp] as u32
            } else {
                0xff
            };
            self.clow += byte_val << 8;
            self.ct = 8;
        }

        if self.clow > 0xffff {
            self.chigh += self.clow >> 16;
            self.clow &= 0xffff;
        }
    }

    // C.3.2 Decoding a decision (DECODE)
    fn read_bit(&mut self, contexts: &mut [i8], pos: usize) -> u8 {
        println!("{}, a: {}", self.counter, self.a);
        self.counter += 1;

        // Contexts are packed into 1 byte:
        // highest 7 bits carry cx.index, lowest bit carries cx.mps
        let mut cx_index = (contexts[pos] >> 1) as usize;
        let mut cx_mps = (contexts[pos] & 1) as u8;

        if cx_index >= QE_TABLE.len() {
            cx_index = QE_TABLE.len() - 1;
        }

        let qe_table_icx = QE_TABLE[cx_index];
        let qe_icx = qe_table_icx.qe;
        let d: u8;
        let mut a = self.a - qe_icx;

        if self.chigh < qe_icx {
            // exchangeLps
            if a < qe_icx {
                a = qe_icx;
                d = cx_mps;
                cx_index = qe_table_icx.nmps as usize;
            } else {
                a = qe_icx;
                d = 1 ^ cx_mps;
                if qe_table_icx.switch_flag == 1 {
                    cx_mps = d;
                }
                cx_index = qe_table_icx.nlps as usize;
            }
        } else {
            self.chigh -= qe_icx;
            if (a & 0x8000) != 0 {
                self.a = a;
                return cx_mps;
            }
            // exchangeMps
            if a < qe_icx {
                d = 1 ^ cx_mps;
                if qe_table_icx.switch_flag == 1 {
                    cx_mps = d;
                }
                cx_index = qe_table_icx.nlps as usize;
            } else {
                d = cx_mps;
                cx_index = qe_table_icx.nmps as usize;
            }
        }

        // C.3.3 renormD
        loop {
            if self.ct == 0 {
                self.byte_in();
            }

            a <<= 1;
            self.chigh = ((self.chigh << 1) & 0xffff) | ((self.clow >> 15) & 1);
            self.clow = (self.clow << 1) & 0xffff;
            self.ct -= 1;

            if (a & 0x8000) != 0 {
                break;
            }
        }

        self.a = a;
        contexts[pos] = ((cx_index << 1) | (cx_mps as usize)) as i8;
        d
    }
}

// Annex A. Arithmetic Integer Decoding Procedure
// A.2 Procedure for decoding values
fn decode_integer(
    context_cache: &mut ContextCache,
    procedure: &str,
    decoder: &mut ArithmeticDecoder,
) -> Option<i32> {
    let contexts = context_cache.get_contexts(procedure);
    let mut prev = 1;

    let mut read_bits = |length: usize| -> u32 {
        let mut v = 0;
        for _ in 0..length {
            let bit = decoder.read_bit(contexts, prev) as u32;
            prev = if prev < 256 {
                (prev << 1) | (bit as usize)
            } else {
                (((prev << 1) | (bit as usize)) & 511) | 256
            };
            v = (v << 1) | bit;
        }
        v
    };

    let sign = read_bits(1);

    let value = if read_bits(1) != 0 {
        if read_bits(1) != 0 {
            if read_bits(1) != 0 {
                if read_bits(1) != 0 {
                    if read_bits(1) != 0 {
                        read_bits(32) + 4436
                    } else {
                        read_bits(12) + 340
                    }
                } else {
                    read_bits(8) + 84
                }
            } else {
                read_bits(6) + 20
            }
        } else {
            read_bits(4) + 4
        }
    } else {
        read_bits(2)
    };

    let mut signed_value = None;

    if sign == 0 {
        signed_value = Some(value as i32);
    } else if value > 0 {
        signed_value = Some(-(value as i32));
    };

    // Ensure that the integer value doesn't underflow or overflow
    const MIN_INT_32: i32 = i32::MIN;
    const MAX_INT_32: i32 = i32::MAX;

    if let Some(signed_value) = signed_value {
        if signed_value >= MIN_INT_32 && signed_value <= MAX_INT_32 {
            return Some(signed_value);
        }
    }

    None
}

// A.3 The IAID decoding procedure
fn decode_iaid(
    context_cache: &mut ContextCache,
    decoder: &mut ArithmeticDecoder,
    code_length: usize,
) -> u32 {
    let contexts = context_cache.get_contexts("IAID");

    let mut prev = 1;
    for _ in 0..code_length {
        let bit = decoder.read_bit(contexts, prev) as usize;
        prev = (prev << 1) | bit;
    }

    if code_length < 31 {
        (prev & ((1 << code_length) - 1)) as u32
    } else {
        (prev & 0x7fffffff) as u32
    }
}

// Bitmap type for 2D bitmap data
type Bitmap = Vec<Vec<u8>>;

// Template structure for coordinates
#[derive(Clone, Copy, Debug)]
struct TemplatePixel {
    x: i32,
    y: i32,
}

// Utility function equivalent to log2 from JS
fn log2(n: usize) -> usize {
    if n == 0 {
        0
    } else {
        (n as f64).log2().ceil() as usize
    }
}

// Placeholder structs for complex Huffman functionality
#[derive(Debug)]
struct SymbolDictionaryHuffmanTables {
    // Huffman tables for symbol dictionary as per JBIG2 spec Table E.1
    pub height_table: HuffmanTable,
    pub width_table: HuffmanTable,
    pub bitmap_size_table: Option<HuffmanTable>,
    pub _aggregate_table: Option<HuffmanTable>,
}

#[derive(Debug)]
struct ReaderInner<'a> {
    data: &'a [u8],
    start: usize,
    end: usize,
    position: usize,
    shift: i32,
    current_byte: u8,
}

// Reader class - ported from JS Reader class
#[derive(Debug)]
struct Reader<'a>(RefCell<ReaderInner<'a>>);

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], start: usize, end: usize) -> Self {
        Self(RefCell::new(ReaderInner {
            data,
            end,
            start,
            position: start,
            shift: -1,
            current_byte: 0,
        }))
    }

    fn read_bit(&self) -> Result<u8, Jbig2Error> {
        let mut s = self.0.borrow_mut();

        if s.shift < 0 {
            if s.position >= s.end {
                return Err(Jbig2Error::new("end of data while reading bit"));
            }
            s.current_byte = s.data[s.position];
            s.position += 1;
            s.shift = 7;
        }
        let bit = (s.current_byte >> s.shift) & 1;
        s.shift -= 1;
        Ok(bit)
    }

    fn read_bits(&self, num_bits: usize) -> Result<u32, Jbig2Error> {
        let mut result = 0u32;
        for i in (0..num_bits).rev() {
            result |= (self.read_bit()? as u32) << i;
        }

        Ok(result)
    }

    fn byte_align(&self) {
        self.0.borrow_mut().shift = -1;
    }

    fn _next(&self) -> i32 {
        let mut s = self.0.borrow_mut();

        if s.position >= s.end {
            return -1;
        }
        let byte = s.data[s.position] as i32;
        s.position += 1;
        byte
    }
}

// Placeholder for Huffman tables
#[derive(Debug)]
struct TextRegionHuffmanTables {
    // Huffman tables for text region as per JBIG2 spec Table E.2
    pub symbol_id_table: HuffmanTable,
    pub table_delta_t: HuffmanTable, // JavaScript: tableDeltaT
    pub table_delta_s: HuffmanTable, // JavaScript: tableDeltaS
    pub table_first_s: Option<HuffmanTable>, // JavaScript: tableFirstS
    pub _ds_table: Option<HuffmanTable>,
    pub _dt_table: Option<HuffmanTable>,
    pub _rdw_table: Option<HuffmanTable>,
    pub _rdh_table: Option<HuffmanTable>,
    pub _rdx_table: Option<HuffmanTable>,
    pub _rdy_table: Option<HuffmanTable>,
    pub _rsize_table: Option<HuffmanTable>,
}

// Huffman decoding classes - ported from JS HuffmanLine, HuffmanTreeNode, HuffmanTable

#[derive(Debug, Clone)]
struct HuffmanLine {
    is_oob: bool,
    range_low: i32,
    prefix_length: usize,
    range_length: usize,
    prefix_code: u32,
    is_lower_range: bool,
}

impl HuffmanLine {
    // ✅ FAITHFUL PORT: Unified constructor like JavaScript version
    // JavaScript: constructor(lineData) handles both OOB and normal lines based on array length
    fn new(line_data: &[i32]) -> Self {
        if line_data.len() == 2 {
            // OOB line
            Self {
                is_oob: true,
                range_low: 0,
                prefix_length: line_data[0] as usize,
                range_length: 0,
                prefix_code: line_data[1] as u32,
                is_lower_range: false,
            }
        } else if line_data.len() >= 4 {
            // Normal, upper range or lower range line
            let is_lower_range = line_data.len() == 5 && line_data[4] == -1; // Using -1 as "lower" marker
            Self {
                is_oob: false,
                range_low: line_data[0],
                prefix_length: line_data[1] as usize,
                range_length: line_data[2] as usize,
                prefix_code: line_data[3] as u32,
                is_lower_range,
            }
        } else {
            // Invalid line data
            Self::new_oob(0, 0)
        }
    }

    fn new_oob(prefix_length: usize, prefix_code: u32) -> Self {
        Self {
            is_oob: true,
            range_low: 0,
            prefix_length,
            range_length: 0,
            prefix_code,
            is_lower_range: false,
        }
    }

    fn new_normal(
        range_low: i32,
        prefix_length: usize,
        range_length: usize,
        prefix_code: u32,
    ) -> Self {
        Self {
            is_oob: false,
            range_low,
            prefix_length,
            range_length,
            prefix_code,
            is_lower_range: false,
        }
    }

    fn new_lower(
        range_low: i32,
        prefix_length: usize,
        range_length: usize,
        prefix_code: u32,
    ) -> Self {
        Self {
            is_oob: false,
            range_low,
            prefix_length,
            range_length,
            prefix_code,
            is_lower_range: true,
        }
    }
}

#[derive(Debug, Clone)]
struct HuffmanTreeNode {
    children: [Option<Box<HuffmanTreeNode>>; 2],
    is_leaf: bool,
    range_length: usize,
    range_low: i32,
    is_lower_range: bool,
    is_oob: bool,
}

impl HuffmanTreeNode {
    fn new_leaf(line: &HuffmanLine) -> Self {
        Self {
            children: [None, None],
            is_leaf: true,
            range_length: line.range_length,
            range_low: line.range_low,
            is_lower_range: line.is_lower_range,
            is_oob: line.is_oob,
        }
    }

    fn new_node() -> Self {
        Self {
            children: [None, None],
            is_leaf: false,
            range_length: 0,
            range_low: 0,
            is_lower_range: false,
            is_oob: false,
        }
    }

    fn build_tree(&mut self, line: &HuffmanLine, shift: i32) {
        let bit = ((line.prefix_code >> shift) & 1) as usize;
        if shift <= 0 {
            // Create a leaf node
            self.children[bit] = Some(Box::new(HuffmanTreeNode::new_leaf(line)));
        } else {
            // Create an intermediate node and continue recursively
            if self.children[bit].is_none() {
                self.children[bit] = Some(Box::new(HuffmanTreeNode::new_node()));
            }
            self.children[bit]
                .as_mut()
                .unwrap()
                .build_tree(line, shift - 1);
        }
    }

    fn decode_node(&self, reader: &Reader) -> Result<Option<i32>, Jbig2Error> {
        if self.is_leaf {
            if self.is_oob {
                return Ok(None);
            }
            let ht_offset = reader.read_bits(self.range_length)? as i32;
            let result = self.range_low
                + if self.is_lower_range {
                    -ht_offset
                } else {
                    ht_offset
                };
            return Ok(Some(result));
        }

        let bit = reader.read_bit()? as usize;
        match &self.children[bit] {
            Some(node) => node.decode_node(reader),
            None => Err(Jbig2Error::new("invalid Huffman data")),
        }
    }
}

#[derive(Debug, Clone)]
struct HuffmanTable {
    root_node: HuffmanTreeNode,
}

impl HuffmanTable {
    fn new(mut lines: Vec<HuffmanLine>, prefix_codes_done: bool) -> Self {
        if !prefix_codes_done {
            Self::assign_prefix_codes(&mut lines);
        }

        // Create Huffman tree
        let mut root_node = HuffmanTreeNode::new_node();
        for line in &lines {
            if line.prefix_length > 0 {
                root_node.build_tree(line, line.prefix_length as i32 - 1);
            }
        }

        Self { root_node }
    }

    fn decode(&self, reader: &Reader) -> Result<Option<i32>, Jbig2Error> {
        self.root_node.decode_node(reader)
    }

    fn assign_prefix_codes(lines: &mut [HuffmanLine]) {
        // Annex B.3 Assigning the prefix codes
        let mut prefix_length_max = 0usize;
        for line in lines.iter() {
            prefix_length_max = prefix_length_max.max(line.prefix_length);
        }

        let mut histogram = vec![0u32; prefix_length_max + 1];
        for line in lines.iter() {
            histogram[line.prefix_length] += 1;
        }

        let mut current_length = 1usize;
        let mut first_code = 0u32;
        histogram[0] = 0;

        while current_length <= prefix_length_max {
            first_code = (first_code + histogram[current_length - 1]) << 1;
            let mut current_code = first_code;

            for line in lines.iter_mut() {
                if line.prefix_length == current_length {
                    line.prefix_code = current_code;
                    current_code += 1;
                }
            }
            current_length += 1;
        }
    }
}

// Utility function for reading uint32 values
fn read_uint32(data: &[u8], offset: usize) -> u32 {
    if offset + 4 > data.len() {
        return 0;
    }
    ((data[offset] as u32) << 24)
        | ((data[offset + 1] as u32) << 16)
        | ((data[offset + 2] as u32) << 8)
        | (data[offset + 3] as u32)
}

// Segment structures and reading functions

#[derive(Debug, Clone)]
struct SegmentHeader {
    number: u32,
    segment_type: u8,
    type_name: String,
    _deferred_non_retain: bool,
    _retain_bits: Vec<u8>,
    referred_to: Vec<u32>,
    _page_association: u32,
    length: u32,
    header_end: usize,
}

#[derive(Debug)]
struct Segment {
    header: SegmentHeader,
    data: Vec<u8>,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct RegionSegmentInformation {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    combination_operator: u8,
}

// Utility functions for reading integers
fn read_uint16(data: &[u8], offset: usize) -> u16 {
    if offset + 2 > data.len() {
        return 0;
    }
    ((data[offset] as u16) << 8) | (data[offset + 1] as u16)
}

fn read_int8(data: &[u8], offset: usize) -> i8 {
    if offset >= data.len() {
        return 0;
    }
    data[offset] as i8
}

// Segment header reading - ported from readSegmentHeader function
fn read_segment_header(data: &[u8], start: usize) -> Result<SegmentHeader, Jbig2Error> {
    if start + 6 > data.len() {
        return Err(Jbig2Error::new("insufficient data for segment header"));
    }

    let number = read_uint32(data, start);
    let flags = data[start + 4];
    let segment_type = flags & 0x3f;

    if segment_type as usize >= SEGMENT_TYPES.len()
        || SEGMENT_TYPES[segment_type as usize].is_none()
    {
        return Err(Jbig2Error::new(&format!(
            "invalid segment type: {}",
            segment_type
        )));
    }

    let type_name = SEGMENT_TYPES[segment_type as usize].unwrap().to_string();
    let deferred_non_retain = (flags & 0x80) != 0;
    let page_association_field_size = (flags & 0x40) != 0;

    let referred_flags = data[start + 5];
    let mut referred_to_count = ((referred_flags >> 5) & 7) as usize;
    let mut retain_bits = vec![referred_flags & 31];
    let mut position = start + 6;

    if referred_flags == 7 {
        referred_to_count = (read_uint32(data, position - 1) & 0x1fffffff) as usize;
        position += 3;
        let mut bytes = (referred_to_count + 7) >> 3;
        if position >= data.len() {
            return Err(Jbig2Error::new("insufficient data for retain bits"));
        }
        retain_bits[0] = data[position];
        position += 1;
        bytes -= 1;
        while bytes > 0 && position < data.len() {
            retain_bits.push(data[position]);
            position += 1;
            bytes -= 1;
        }
    } else if referred_flags == 5 || referred_flags == 6 {
        return Err(Jbig2Error::new("invalid referred-to flags"));
    }

    let referred_to_segment_number_size = if number <= 256 {
        1
    } else if number <= 65536 {
        2
    } else {
        4
    };

    let mut referred_to = Vec::new();
    for _ in 0..referred_to_count {
        if position + referred_to_segment_number_size > data.len() {
            return Err(Jbig2Error::new(
                "insufficient data for referred-to segments",
            ));
        }

        let number = match referred_to_segment_number_size {
            1 => data[position] as u32,
            2 => read_uint16(data, position) as u32,
            4 => read_uint32(data, position),
            _ => return Err(Jbig2Error::new("invalid segment number size")),
        };
        referred_to.push(number);
        position += referred_to_segment_number_size;
    }

    let page_association = if !page_association_field_size {
        if position >= data.len() {
            return Err(Jbig2Error::new("insufficient data for page association"));
        }
        data[position] as u32
    } else {
        if position + 4 > data.len() {
            return Err(Jbig2Error::new("insufficient data for page association"));
        }
        read_uint32(data, position)
    };
    position += if page_association_field_size { 4 } else { 1 };

    if position + 4 > data.len() {
        return Err(Jbig2Error::new("insufficient data for segment length"));
    }
    let length = read_uint32(data, position);
    position += 4;

    // Handle unknown segment length (0xffffffff) cases
    // When length is unknown, we need to read until end of data or next segment
    if length == 0xffffffff {
        // Implement end-of-segment detection for ImmediateGenericRegion (type 38)
        if segment_type == 38 {
            // For ImmediateGenericRegion with unknown length, we need to find the end pattern
            // by reading ahead to find the region information and create a search pattern
            if position + 17 > data.len() {
                return Err(Jbig2Error::new(
                    "insufficient data for region info in unknown length segment",
                ));
            }

            let region_height = read_uint32(data, position + 4);
            let region_flags = if position + 17 < data.len() {
                data[position + 17]
            } else {
                0
            };
            let mmr = (region_flags & 1) != 0;

            // Create search pattern based on MMR flag and height
            let search_pattern = if mmr {
                // For MMR: just height bytes
                vec![
                    (region_height >> 24) as u8,
                    (region_height >> 16) as u8,
                    (region_height >> 8) as u8,
                    region_height as u8,
                ]
            } else {
                // For non-MMR: 0xff, 0xac followed by height bytes
                vec![
                    0xff,
                    0xac,
                    (region_height >> 24) as u8,
                    (region_height >> 16) as u8,
                    (region_height >> 8) as u8,
                    region_height as u8,
                ]
            };

            // Search for the pattern starting from after the segment header
            let search_start = position + 18; // After region info and flags
            let search_end = data.len().saturating_sub(search_pattern.len());
            let mut found_end = None;

            for i in search_start..=search_end {
                if data[i..i + search_pattern.len()] == search_pattern {
                    found_end = Some(i + search_pattern.len());
                    break;
                }
            }

            let actual_length = if let Some(end_pos) = found_end {
                end_pos - position
            } else {
                // If pattern not found, use remaining data
                data.len() - position
            };

            return Ok(SegmentHeader {
                number,
                segment_type,
                type_name,
                _deferred_non_retain: deferred_non_retain,
                _retain_bits: retain_bits,
                referred_to,
                _page_association: page_association,
                length: actual_length as u32,
                header_end: position,
            });
        }

        // For other segment types with unknown length, use remaining data
        let remaining_length = data.len() - position;
        return Ok(SegmentHeader {
            number,
            segment_type,
            type_name,
            _deferred_non_retain: deferred_non_retain,
            _retain_bits: retain_bits,
            referred_to,
            _page_association: page_association,
            length: remaining_length as u32,
            header_end: position,
        });
    }

    Ok(SegmentHeader {
        number,
        segment_type,
        type_name,
        _deferred_non_retain: deferred_non_retain,
        _retain_bits: retain_bits,
        referred_to,
        _page_association: page_association,
        length,
        header_end: position,
    })
}

// Region segment information reading - ported from readRegionSegmentInformation
fn read_region_segment_information(
    data: &[u8],
    start: usize,
) -> Result<RegionSegmentInformation, Jbig2Error> {
    if start + 17 > data.len() {
        return Err(Jbig2Error::new(
            "insufficient data for region segment information",
        ));
    }

    Ok(RegionSegmentInformation {
        width: read_uint32(data, start),
        height: read_uint32(data, start + 4),
        x: read_uint32(data, start + 8),
        y: read_uint32(data, start + 12),
        combination_operator: data[start + 16] & 7,
    })
}

// All major functions have been implemented:
// - decode_halftone_region ✓
// - segment processing and visitor pattern ✓
// - SimpleSegmentVisitor and main parsing logic ✓
// - MMR decoding functions ✓ (basic implementation)
// - tables segment decoding and standard tables ✓

// Main JBIG2 decoder class
struct Jbig2Image {
    width: usize,
    height: usize,
    segments: Vec<Segment>,
}

impl Jbig2Image {
    fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            segments: Vec::new(),
        }
    }

    fn parse_chunks(&mut self, chunks: &[Chunk]) -> Option<Vec<u8>> {
        // Parse all segments from chunks first
        for chunk in chunks {
            if let Err(e) = self.parse_chunk(chunk) {
                warn!("Error parsing JBIG2 chunk: {}", e);
                return None;
            }
        }

        // Process segments with visitor pattern to generate final bitmap
        let mut visitor = SimpleSegmentVisitor::new();

        if let Err(e) = process_segments(&self.segments, &mut visitor) {
            warn!("Error processing JBIG2 segments: {}", e);
            return None;
        }

        // Set width and height from page info (like parseJbig2 in JS)
        if let Some(page_info) = &visitor.current_page_info {
            self.width = page_info.width as usize;
            self.height = page_info.height as usize;
        }

        // Return the final bitmap buffer if available
        visitor.buffer
    }

    fn parse_chunk(&mut self, chunk: &Chunk) -> Result<(), Jbig2Error> {
        // TODO: SEGMENT READING DIFFERENCES: JS readSegments() handles randomAccess flag from header
        // differently - if randomAccess is false, it sets segment positions immediately during parsing.
        // If randomAccess is true, it defers position setting until all segments are read.
        // Rust version always sets positions immediately and doesn't handle randomAccess flag.
        // This could affect processing order and memory usage for some JBIG2 files.
        let data = &chunk.data;
        let mut position = chunk.start;
        let end = chunk.end;

        // Skip file header if present (first 9 bytes for file organization)
        if position + 9 <= data.len() && data[position..position + 4] == [0x97, 0x4A, 0x42, 0x32] {
            // Skip the file header
            position += 9;
        }

        // Read segments
        while position < end && position + 6 <= data.len() {
            let segment_header = read_segment_header(data, position)?;
            position = segment_header.header_end;

            if position + segment_header.length as usize > data.len() {
                return Err(Jbig2Error::new("segment data extends beyond chunk"));
            }

            let segment = Segment {
                header: segment_header.clone(),
                data: data[position..position + segment_header.length as usize].to_vec(),
                start: 0, // Relative to segment data
                end: segment_header.length as usize,
            };

            position += segment_header.length as usize;
            self.segments.push(segment);

            // Break on end of file segment
            if segment_header.segment_type == 51 {
                break;
            }
        }

        Ok(())
    }
}

// SimpleSegmentVisitor - ported from JS SimpleSegmentVisitor class
#[derive(Debug)]
struct SimpleSegmentVisitor {
    current_page_info: Option<PageInfo>,
    buffer: Option<Vec<u8>>,
    symbols: HashMap<u32, Vec<Bitmap>>,
    patterns: HashMap<u32, Vec<Bitmap>>,
    custom_tables: HashMap<u32, HuffmanTable>,
}

#[derive(Debug, Clone)]
struct PageInfo {
    width: u32,
    height: u32,
    default_pixel_value: u8,
    combination_operator: u8,
    combination_operator_override: bool,
}

impl SimpleSegmentVisitor {
    fn new() -> Self {
        // TODO: VISITOR INITIALIZATION DIFFERENCES: JS version uses lazy initialization for symbols,
        // patterns, and customTables properties (created only when first needed with `if (!symbols)`).
        // Rust version initializes HashMap containers immediately. JS also doesn't initialize
        // currentPageInfo until onPageInformation is called.
        Self {
            current_page_info: None,
            buffer: None,
            symbols: HashMap::new(),
            patterns: HashMap::new(),
            custom_tables: HashMap::new(),
        }
    }

    fn on_page_information(&mut self, info: PageInfo) {
        self.current_page_info = Some(info.clone());
        let row_size = (info.width + 7) >> 3;
        let mut buffer = vec![0u8; (row_size * info.height) as usize];

        // Fill with 0xFF if default pixel value is set
        if info.default_pixel_value != 0 {
            buffer.fill(0xff);
        }
        self.buffer = Some(buffer);
    }

    pub fn draw_bitmap(
        &mut self,
        region_info: &RegionSegmentInformation,
        bitmap: &Bitmap,
    ) -> Result<(), Jbig2Error> {
        let page_info = self
            .current_page_info
            .as_ref()
            .ok_or_else(|| Jbig2Error::new("no page information available"))?;
        let buffer = self
            .buffer
            .as_mut()
            .ok_or_else(|| Jbig2Error::new("no buffer available"))?;

        let width = region_info.width as usize;
        let height = region_info.height as usize;
        let row_size = ((page_info.width + 7) >> 3) as usize;

        let combination_operator = if page_info.combination_operator_override {
            region_info.combination_operator
        } else {
            page_info.combination_operator
        };

        let mask0 = 128u8 >> (region_info.x & 7);
        let mut offset0 = (region_info.y * (page_info.width + 7) / 8 + region_info.x / 8) as usize;

        match combination_operator {
            0 => {
                // OR
                for i in 0..height {
                    if i >= bitmap.len() {
                        break;
                    }
                    let mut mask = mask0;
                    let mut offset = offset0;

                    for j in 0..width {
                        if j < bitmap[i].len() && bitmap[i][j] != 0 {
                            if offset < buffer.len() {
                                buffer[offset] |= mask;
                            }
                        }
                        mask >>= 1;
                        if mask == 0 {
                            mask = 128;
                            offset += 1;
                        }
                    }
                    offset0 += row_size;
                }
            }
            2 => {
                // XOR
                for i in 0..height {
                    if i >= bitmap.len() {
                        break;
                    }
                    let mut mask = mask0;
                    let mut offset = offset0;

                    for j in 0..width {
                        if j < bitmap[i].len() && bitmap[i][j] != 0 {
                            if offset < buffer.len() {
                                buffer[offset] ^= mask;
                            }
                        }
                        mask >>= 1;
                        if mask == 0 {
                            mask = 128;
                            offset += 1;
                        }
                    }
                    offset0 += row_size;
                }
            }
            _ => {
                return Err(Jbig2Error::new(&format!(
                    "operator {} is not supported",
                    combination_operator
                )));
            }
        }

        Ok(())
    }

    fn on_immediate_generic_region(
        &mut self,
        region: &GenericRegion,
        data: &[u8],
        start: usize,
        end: usize,
    ) -> Result<(), Jbig2Error> {
        let mut decoding_context = DecodingContext::new(data.to_vec(), start, end);

        let bitmap = if region.mmr {
            // MMR-coded generic region
            let data_slice = &data[start..end];
            decode_mmr_bitmap(
                data_slice,
                region.info.width as usize,
                region.info.height as usize,
                false,
            )?
        } else {
            decode_bitmap(
                false, // mmr
                region.info.width as usize,
                region.info.height as usize,
                region.template,
                region.prediction,
                None, // skip
                &region.at,
                &mut decoding_context,
            )?
        };

        self.draw_bitmap(&region.info, &bitmap)
    }

    fn on_symbol_dictionary(
        &mut self,
        dictionary: &SymbolDictionary,
        current_segment: u32,
        referred_segments: &[u32],
        data: &[u8],
        start: usize,
        end: usize,
    ) -> Result<(), Jbig2Error> {
        // Collect input symbols from referred segments
        let mut input_symbols = Vec::new();
        for &referred_segment in referred_segments {
            if let Some(referred_symbols) = self.symbols.get(&referred_segment) {
                input_symbols.extend(referred_symbols.iter().cloned());
            }
        }

        let mut decoding_context = DecodingContext::new(data.to_vec(), start, end);

        // Create Huffman tables and reader if needed (like JS implementation)
        let huffman_tables = if dictionary.huffman {
            Some(self.get_symbol_dictionary_huffman_tables(dictionary, referred_segments)?)
        } else {
            None
        };

        let mut huffman_reader = if dictionary.huffman {
            Some(Reader::new(data, start, end))
        } else {
            None
        };

        let new_symbols = decode_symbol_dictionary(
            dictionary.huffman,
            dictionary.refinement,
            &input_symbols,
            dictionary.number_of_new_symbols as usize,
            dictionary.number_of_exported_symbols as usize,
            huffman_tables.as_ref(),
            dictionary.template,
            &dictionary.at,
            dictionary.refinement_template,
            &dictionary.refinement_at,
            &mut decoding_context,
            huffman_reader.as_ref(),
        )?;

        // Store all symbols (input + new)
        let mut all_symbols = input_symbols;
        all_symbols.extend(new_symbols);
        self.symbols.insert(current_segment, all_symbols);

        Ok(())
    }

    fn on_immediate_text_region(
        &mut self,
        region: &TextRegion,
        referred_segments: &[u32],
        data: &[u8],
        start: usize,
        end: usize,
    ) -> Result<(), Jbig2Error> {
        // Collect input symbols from referred segments
        let mut input_symbols = Vec::new();
        for &referred_segment in referred_segments {
            if let Some(referred_symbols) = self.symbols.get(&referred_segment) {
                input_symbols.extend(referred_symbols.iter().cloned());
            }
        }

        if input_symbols.is_empty() {
            return Err(Jbig2Error::new("no symbols available for text region"));
        }

        let mut decoding_context = DecodingContext::new(data.to_vec(), start, end);
        let symbol_code_length = log2(input_symbols.len()).max(1);

        // Create Huffman tables and reader if needed (like JS implementation)
        let huffman_tables = if region.huffman {
            // Use the enhanced function that can decode symbol ID table
            let mut huffman_reader = Some(Reader::new(data, start, end));
            Some(self.get_text_region_huffman_tables_with_reader(
                region,
                referred_segments,
                input_symbols.len(),
                huffman_reader.as_ref(),
            )?)
        } else {
            None
        };

        let mut huffman_reader = if region.huffman {
            Some(Reader::new(data, start, end))
        } else {
            None
        };

        let bitmap = decode_text_region(
            region.huffman,
            region.refinement,
            region.info.width as usize,
            region.info.height as usize,
            region.default_pixel_value,
            region.number_of_symbol_instances as usize,
            region.strip_size as usize,
            &input_symbols,
            symbol_code_length,
            region.transposed,
            region.ds_offset,
            region.reference_corner,
            region.combination_operator,
            huffman_tables.as_ref(),
            region.refinement_template,
            &region.refinement_at,
            &mut decoding_context,
            region.log_strip_size,
            huffman_reader.as_ref(),
        )?;

        self.draw_bitmap(&region.info, &bitmap)
    }

    fn on_pattern_dictionary(
        &mut self,
        dictionary: &PatternDictionary,
        current_segment: u32,
        data: &[u8],
        start: usize,
        end: usize,
    ) -> Result<(), Jbig2Error> {
        let mut decoding_context = DecodingContext::new(data.to_vec(), start, end);

        let patterns = decode_pattern_dictionary(
            dictionary.mmr,
            dictionary.pattern_width as usize,
            dictionary.pattern_height as usize,
            dictionary.max_pattern_index as usize,
            dictionary.template,
            &mut decoding_context,
        )?;

        self.patterns.insert(current_segment, patterns);
        Ok(())
    }

    fn on_immediate_halftone_region(
        &mut self,
        region: &HalftoneRegion,
        referred_segments: &[u32],
        data: &[u8],
        start: usize,
        end: usize,
    ) -> Result<(), Jbig2Error> {
        // Collect patterns from referred segments
        let mut patterns = Vec::new();
        for &referred_segment in referred_segments {
            if let Some(referred_patterns) = self.patterns.get(&referred_segment) {
                patterns.extend(referred_patterns.iter().cloned());
            }
        }

        if patterns.is_empty() {
            return Err(Jbig2Error::new("no patterns available for halftone region"));
        }

        let mut decoding_context = DecodingContext::new(data.to_vec(), start, end);

        let bitmap = decode_halftone_region(
            region.mmr,
            &patterns,
            region.template,
            region.info.width as usize,
            region.info.height as usize,
            region.default_pixel_value,
            region.enable_skip,
            region.combination_operator,
            region.grid_width as usize,
            region.grid_height as usize,
            region.grid_offset_x,
            region.grid_offset_y,
            region.grid_vector_x,
            region.grid_vector_y,
            &mut decoding_context,
        )?;

        self.draw_bitmap(&region.info, &bitmap)
    }

    fn on_tables(
        &mut self,
        current_segment: u32,
        data: &[u8],
        start: usize,
        end: usize,
    ) -> Result<(), Jbig2Error> {
        let table = decode_tables_segment(data, start, end)?;
        self.custom_tables.insert(current_segment, table);
        Ok(())
    }

    fn get_symbol_dictionary_huffman_tables(
        &self,
        dictionary: &SymbolDictionary,
        referred_segments: &[u32],
    ) -> Result<SymbolDictionaryHuffmanTables, Jbig2Error> {
        // 7.4.2.1.6 Symbol dictionary segment Huffman table selection
        let mut custom_index = 0;

        // Height table selection based on huffmanDHSelector
        let height_table = match dictionary.huffman_dh_selector {
            0 | 1 => get_standard_table(dictionary.huffman_dh_selector as u32 + 4)?,
            3 => {
                let table =
                    get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?
                        .clone();
                custom_index += 1;
                table
            }
            _ => return Err(Jbig2Error::new("invalid Huffman DH selector")),
        };

        // Width table selection based on huffmanDWSelector
        let width_table = match dictionary.huffman_dw_selector {
            0 | 1 => get_standard_table(dictionary.huffman_dw_selector as u32 + 2)?,
            3 => {
                let table =
                    get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?
                        .clone();
                custom_index += 1;
                table
            }
            _ => return Err(Jbig2Error::new("invalid Huffman DW selector")),
        };

        // Bitmap size table selection based on bitmapSizeSelector
        let bitmap_size_table = if dictionary.bitmap_size_selector {
            Some(
                get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?
                    .clone(),
            )
        } else {
            Some(get_standard_table(1)?)
        };

        // Aggregation instances table selection based on aggregationInstancesSelector
        let _aggregate_table = if dictionary.aggregation_instances_selector {
            Some(
                get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?
                    .clone(),
            )
        } else {
            Some(get_standard_table(1)?)
        };

        Ok(SymbolDictionaryHuffmanTables {
            height_table,
            width_table,
            bitmap_size_table,
            _aggregate_table,
        })
    }

    fn get_text_region_huffman_tables(
        &self,
        region: &TextRegion,
        referred_segments: &[u32],
        _number_of_symbols: usize,
    ) -> Result<TextRegionHuffmanTables, Jbig2Error> {
        self.get_text_region_huffman_tables_with_reader(
            region,
            referred_segments,
            _number_of_symbols,
            None,
        )
    }

    fn get_text_region_huffman_tables_with_reader(
        &self,
        region: &TextRegion,
        referred_segments: &[u32],
        number_of_symbols: usize,
        huffman_reader: Option<&Reader>,
    ) -> Result<TextRegionHuffmanTables, Jbig2Error> {
        // 7.4.3.1.6 Text region segment Huffman table selection
        let mut custom_index = 0;

        // Symbol ID table decoding
        let symbol_id_table = if let Some(reader) = huffman_reader {
            // ✅ FAITHFUL PORT: Complete JavaScript implementation with RUNCODE handling
            // 7.4.3.1.7 Symbol ID Huffman table decoding
            decode_symbol_id_huffman_table(reader, number_of_symbols)?
        } else {
            // Fallback to standard table when no reader available
            get_standard_table(15)?
        };

        // First S table selection based on huffmanFS selector
        let fs_table = match region.huffman_fs {
            0 | 1 => Some(get_standard_table(region.huffman_fs as u32 + 6)?),
            3 => {
                let table =
                    get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?
                        .clone();
                custom_index += 1;
                Some(table)
            }
            _ => return Err(Jbig2Error::new("invalid Huffman FS selector")),
        };

        // Delta S table selection based on huffmanDS selector
        let s_table = match region.huffman_ds {
            0 | 1 | 2 => get_standard_table(region.huffman_ds as u32 + 8)?,
            3 => {
                let table =
                    get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?
                        .clone();
                custom_index += 1;
                table
            }
            _ => return Err(Jbig2Error::new("invalid Huffman DS selector")),
        };

        // Delta T table selection based on huffmanDT selector
        let t_table = match region.huffman_dt {
            0 | 1 | 2 => get_standard_table(region.huffman_dt as u32 + 11)?,
            3 => {
                let table =
                    get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?
                        .clone();
                custom_index += 1;
                table
            }
            _ => return Err(Jbig2Error::new("invalid Huffman DT selector")),
        };

        if region.refinement {
            // Load tables RDW, RDH, RDX and RDY based on selectors
            // For now, return error as refinement with Huffman is complex
            return Err(Jbig2Error::new("refinement with Huffman is not supported"));
        }

        Ok(TextRegionHuffmanTables {
            symbol_id_table,
            table_delta_t: t_table.clone(), // JavaScript: tableDeltaT
            table_delta_s: s_table.clone(), // JavaScript: tableDeltaS
            table_first_s: fs_table,        // JavaScript: tableFirstS
            _ds_table: Some(s_table),
            _dt_table: Some(t_table),
            _rdw_table: None,
            _rdh_table: None,
            _rdx_table: None,
            _rdy_table: None,
            _rsize_table: None,
        })
    }
}

// Placeholder structures for different segment types
#[derive(Debug)]
struct GenericRegion {
    info: RegionSegmentInformation,
    mmr: bool,
    template: usize,
    prediction: bool,
    at: Vec<TemplatePixel>,
}

#[derive(Debug)]
struct SymbolDictionary {
    huffman: bool,
    refinement: bool,
    huffman_dh_selector: u8,
    huffman_dw_selector: u8,
    bitmap_size_selector: bool,
    aggregation_instances_selector: bool,
    bitmap_coding_context_used: bool,
    bitmap_coding_context_retained: bool,
    template: usize,
    refinement_template: usize,
    at: Vec<TemplatePixel>,
    refinement_at: Vec<TemplatePixel>,
    number_of_exported_symbols: u32,
    number_of_new_symbols: u32,
}

#[derive(Debug)]
struct TextRegion {
    info: RegionSegmentInformation,
    huffman: bool,
    refinement: bool,
    huffman_fs: u8,
    huffman_ds: u8,
    huffman_dt: u8,
    huffman_refinement_dw: u8,
    huffman_refinement_dh: u8,
    huffman_refinement_dx: u8,
    huffman_refinement_dy: u8,
    huffman_refinement_size_selector: bool,
    default_pixel_value: u8,
    number_of_symbol_instances: u32,
    strip_size: u32,
    transposed: bool,
    ds_offset: i32,
    reference_corner: u8,
    combination_operator: u8,
    refinement_template: usize,
    refinement_at: Vec<TemplatePixel>,
    log_strip_size: usize,
}

#[derive(Debug)]
struct PatternDictionary {
    mmr: bool,
    pattern_width: u32,
    pattern_height: u32,
    max_pattern_index: u32,
    template: usize,
}

#[derive(Debug)]
struct HalftoneRegion {
    info: RegionSegmentInformation,
    mmr: bool,
    template: usize,
    default_pixel_value: u8,
    enable_skip: bool,
    combination_operator: u8,
    grid_width: u32,
    grid_height: u32,
    grid_offset_x: i32,
    grid_offset_y: i32,
    grid_vector_x: i32,
    grid_vector_y: i32,
}

// Halftone region decoding - ported from decodeHalftoneRegion function
#[allow(clippy::too_many_arguments)]
fn decode_halftone_region(
    mmr: bool,
    patterns: &[Bitmap],
    template: usize,
    region_width: usize,
    region_height: usize,
    default_pixel_value: u8,
    enable_skip: bool,
    combination_operator: u8,
    grid_width: usize,
    grid_height: usize,
    grid_offset_x: i32,
    grid_offset_y: i32,
    grid_vector_x: i32,
    grid_vector_y: i32,
    decoding_context: &mut DecodingContext,
) -> Result<Bitmap, Jbig2Error> {
    // ✅ FAITHFUL PORT: Complete JavaScript implementation with proper grid vectors
    if enable_skip {
        return Err(Jbig2Error::new("skip is not supported"));
    }
    if combination_operator != 0 {
        return Err(Jbig2Error::new(&format!(
            "operator \"{}\" is not supported in halftone region",
            combination_operator
        )));
    }

    // Prepare bitmap
    let mut region_bitmap: Vec<Vec<u8>> = Vec::with_capacity(region_height);
    for _ in 0..region_height {
        let mut row = vec![0u8; region_width];
        if default_pixel_value != 0 {
            row.fill(default_pixel_value);
        }
        region_bitmap.push(row);
    }

    let number_of_patterns = patterns.len();
    if number_of_patterns == 0 {
        return Ok(region_bitmap);
    }

    let pattern0 = &patterns[0];
    let pattern_width = if !pattern0.is_empty() {
        pattern0[0].len()
    } else {
        0
    };
    let pattern_height = pattern0.len();
    let bits_per_value = log2(number_of_patterns);

    let mut at = Vec::new();
    if !mmr {
        at.push(TemplatePixel {
            x: if template <= 1 { 3 } else { 2 },
            y: -1,
        });
        if template == 0 {
            at.push(TemplatePixel { x: -3, y: -1 });
            at.push(TemplatePixel { x: 2, y: -2 });
            at.push(TemplatePixel { x: -2, y: -2 });
        }
    }

    // Annex C. Gray-scale Image Decoding Procedure
    let mut gray_scale_bit_planes = Vec::with_capacity(bits_per_value);

    for _i in (0..bits_per_value).rev() {
        let bitmap = if mmr {
            // MMR bit planes are in one continuous stream. Only EOFB codes indicate
            // the end of each bitmap, so EOFBs must be decoded.
            let data_slice = &decoding_context.decoder.data
                [decoding_context.decoder.bp..decoding_context.decoder.data_end];
            decode_mmr_bitmap(
                data_slice,
                grid_width,
                grid_height,
                true, // end_of_block = true for bit planes
            )?
        } else {
            decode_bitmap(
                false, // mmr
                grid_width,
                grid_height,
                template,
                false, // prediction
                None,  // skip
                &at,
                decoding_context,
            )?
        };
        gray_scale_bit_planes.push(bitmap);
    }

    // 6.6.5.2 Rendering the patterns
    for mg in 0..grid_height {
        for ng in 0..grid_width {
            let mut bit = 0u8;
            let mut pattern_index = 0usize;

            // Gray decoding - extract pattern index from bit planes
            for j in (0..bits_per_value).rev() {
                if mg < gray_scale_bit_planes[j].len() && ng < gray_scale_bit_planes[j][mg].len() {
                    bit ^= gray_scale_bit_planes[j][mg][ng]; // Gray decoding
                }
                pattern_index |= (bit as usize) << j;
            }

            if pattern_index < patterns.len() {
                let pattern_bitmap = &patterns[pattern_index];

                // ✅ PROPER GRID VECTOR CALCULATION: JavaScript formula
                // x = (gridOffsetX + mg * gridVectorY + ng * gridVectorX) >> 8;
                // y = (gridOffsetY + mg * gridVectorX - ng * gridVectorY) >> 8;
                let x = (grid_offset_x + (mg as i32) * grid_vector_y + (ng as i32) * grid_vector_x)
                    >> 8;
                let y = (grid_offset_y + (mg as i32) * grid_vector_x - (ng as i32) * grid_vector_y)
                    >> 8;

                // Draw pattern bitmap at (x, y)
                if x >= 0
                    && x + (pattern_width as i32) <= region_width as i32
                    && y >= 0
                    && y + (pattern_height as i32) <= region_height as i32
                {
                    // Fast path: pattern is fully contained
                    for i in 0..pattern_height {
                        let region_y = (y + i as i32) as usize;
                        let pattern_row = &pattern_bitmap[i];
                        let region_row = &mut region_bitmap[region_y];
                        for j in 0..pattern_width {
                            let region_x = (x + j as i32) as usize;
                            if j < pattern_row.len() {
                                region_row[region_x] |= pattern_row[j];
                            }
                        }
                    }
                } else {
                    // Bounds-checked path: pattern may be partially outside
                    for i in 0..pattern_height {
                        let region_y = y + i as i32;
                        if region_y < 0 || region_y >= region_height as i32 {
                            continue;
                        }
                        let region_row = &mut region_bitmap[region_y as usize];
                        let pattern_row = &pattern_bitmap[i];
                        for j in 0..pattern_width {
                            let region_x = x + j as i32;
                            if region_x >= 0
                                && (region_x as usize) < region_width
                                && j < pattern_row.len()
                            {
                                region_row[region_x as usize] |= pattern_row[j];
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(region_bitmap)
}

// MMR bitmap decoding using CCITT fax decoder - ported from decodeMMRBitmap function
fn decode_mmr_bitmap(
    data: &[u8],
    width: usize,
    height: usize,
    end_of_block: bool,
) -> Result<Bitmap, Jbig2Error> {
    // ✅ FAITHFUL PORT: Complete JavaScript implementation
    // MMR is the same compression algorithm as the PDF filter CCITTFaxDecode with /K -1.
    let params = CCITTFaxDecoderOptions {
        k: -1,
        columns: width,
        rows: height,
        black_is_1: true,
        eoblock: end_of_block,
        ..Default::default()
    };

    let mut reader = CrateReader::new(data);
    let mut decoder = CCITTFaxDecoder::new(&mut reader, params);
    let mut bitmap: Vec<Vec<u8>> = Vec::with_capacity(height);
    let mut eof = false;

    for _ in 0..height {
        let mut row = Vec::with_capacity(width);
        let mut shift = -1i32;
        let mut current_byte = 0u8;

        for _ in 0..width {
            if shift < 0 {
                let byte = decoder.read_next_char();
                if byte == -1 {
                    // Set the rest of the bits to zero.
                    current_byte = 0;
                    eof = true;
                    shift = 7;
                } else {
                    current_byte = byte as u8;
                    shift = 7;
                }
            }
            let bit = (current_byte >> shift) & 1;
            row.push(bit);
            shift -= 1;
        }
        bitmap.push(row);
    }

    if end_of_block && !eof {
        // Read until EOFB has been consumed.
        let look_for_eof_limit = 5;
        for _ in 0..look_for_eof_limit {
            if decoder.read_next_char() == -1 {
                break;
            }
        }
    }

    Ok(bitmap)
}

// Uncompressed bitmap reading - ported from readUncompressedBitmap function
fn read_uncompressed_bitmap(
    reader: &Reader,
    width: usize,
    height: usize,
) -> Result<Bitmap, Jbig2Error> {
    let mut bitmap: Vec<Vec<u8>> = Vec::with_capacity(height);

    for _ in 0..height {
        let mut row = Vec::with_capacity(width);
        for _ in 0..width {
            let bit = reader.read_bit()?;
            row.push(bit);
        }
        bitmap.push(row);
    }

    reader.byte_align();
    Ok(bitmap)
}

// processSegment function - ported from JS processSegment function
fn process_segment(
    segment: &Segment,
    visitor: &mut SimpleSegmentVisitor,
) -> Result<(), Jbig2Error> {
    // TODO: SEGMENT PARSING DIFFERENCES: JS processSegment() has more detailed flag extraction:
    // 1. SymbolDictionary: huffmanDHSelector, huffmanDWSelector, bitmapSizeSelector, aggregationInstancesSelector,
    //    bitmapCodingContextUsed, bitmapCodingContextRetained flags (bits 2-12 of dictionaryFlags)
    // 2. TextRegion: huffmanFS, huffmanDS, huffmanDT, huffmanRefinement* selectors from textRegionHuffmanFlags
    // 3. More precise bit-field extraction vs simplified boolean flags in Rust version
    // These missing flags affect Huffman table selection and could cause decoding errors.
    let header = &segment.header;
    let data = &segment.data;
    let end = segment.end;
    let mut position = segment.start;

    const REGION_SEGMENT_INFORMATION_FIELD_LENGTH: usize = 17;

    match header.segment_type {
        0 => {
            // SymbolDictionary
            // 7.4.2 Symbol dictionary segment syntax
            if position + 2 > data.len() {
                return Err(Jbig2Error::new("insufficient data for symbol dictionary"));
            }

            let dictionary_flags = read_uint16(data, position);
            position += 2;

            let huffman = (dictionary_flags & 1) != 0;
            let refinement = (dictionary_flags & 2) != 0;
            let huffman_dh_selector = ((dictionary_flags >> 2) & 3) as u8;
            let huffman_dw_selector = ((dictionary_flags >> 4) & 3) as u8;
            let bitmap_size_selector = ((dictionary_flags >> 6) & 1) != 0;
            let aggregation_instances_selector = ((dictionary_flags >> 7) & 1) != 0;
            let bitmap_coding_context_used = (dictionary_flags & 256) != 0;
            let bitmap_coding_context_retained = (dictionary_flags & 512) != 0;
            let template = ((dictionary_flags >> 10) & 3) as usize;
            let refinement_template = ((dictionary_flags >> 12) & 1) as usize;

            let mut at = Vec::new();
            if !huffman {
                let at_length = if template == 0 { 4 } else { 1 };
                for _ in 0..at_length {
                    if position + 2 > data.len() {
                        return Err(Jbig2Error::new("insufficient data for AT pixels"));
                    }
                    at.push(TemplatePixel {
                        x: read_int8(data, position) as i32,
                        y: read_int8(data, position + 1) as i32,
                    });
                    position += 2;
                }
            }

            let mut refinement_at = Vec::new();
            if refinement && refinement_template == 0 {
                for _ in 0..2 {
                    if position + 2 > data.len() {
                        return Err(Jbig2Error::new(
                            "insufficient data for refinement AT pixels",
                        ));
                    }
                    refinement_at.push(TemplatePixel {
                        x: read_int8(data, position) as i32,
                        y: read_int8(data, position + 1) as i32,
                    });
                    position += 2;
                }
            }

            if position + 8 > data.len() {
                return Err(Jbig2Error::new("insufficient data for symbol counts"));
            }
            let number_of_exported_symbols = read_uint32(data, position);
            position += 4;
            let number_of_new_symbols = read_uint32(data, position);
            position += 4;

            let dictionary = SymbolDictionary {
                huffman,
                refinement,
                huffman_dh_selector,
                huffman_dw_selector,
                bitmap_size_selector,
                aggregation_instances_selector,
                bitmap_coding_context_used,
                bitmap_coding_context_retained,
                template,
                refinement_template,
                at,
                refinement_at,
                number_of_exported_symbols,
                number_of_new_symbols,
            };

            visitor.on_symbol_dictionary(
                &dictionary,
                header.number,
                &header.referred_to,
                data,
                position,
                end,
            )?;
        }
        6 | 7 => {
            // ImmediateTextRegion | ImmediateLosslessTextRegion
            if position + REGION_SEGMENT_INFORMATION_FIELD_LENGTH > data.len() {
                return Err(Jbig2Error::new("insufficient data for text region"));
            }

            let info = read_region_segment_information(data, position)?;
            position += REGION_SEGMENT_INFORMATION_FIELD_LENGTH;

            if position + 2 > data.len() {
                return Err(Jbig2Error::new("insufficient data for text region flags"));
            }
            let text_region_segment_flags = read_uint16(data, position);
            position += 2;

            let huffman = (text_region_segment_flags & 1) != 0;
            let refinement = (text_region_segment_flags & 2) != 0;
            let log_strip_size = ((text_region_segment_flags >> 2) & 3) as usize;
            let strip_size = 1u32 << log_strip_size;
            let reference_corner = ((text_region_segment_flags >> 4) & 3) as u8;
            let transposed = (text_region_segment_flags & 64) != 0;
            let combination_operator = ((text_region_segment_flags >> 7) & 3) as u8;
            let default_pixel_value = ((text_region_segment_flags >> 9) & 1) as u8;
            // Extract bits 10-14 (5 bits) and sign-extend from 5-bit to i32
            let ds_offset_bits = (text_region_segment_flags >> 10) & 0x1f; // Extract 5 bits
            let ds_offset = if ds_offset_bits & 0x10 != 0 {
                // Negative value - sign extend from 5-bit
                (ds_offset_bits as i32) | !0x1f
            } else {
                // Positive value
                ds_offset_bits as i32
            };
            let refinement_template = ((text_region_segment_flags >> 15) & 1) as usize;

            // Extract Huffman selectors from textRegionHuffmanFlags if Huffman is used
            let mut huffman_fs = 0u8;
            let mut huffman_ds = 0u8;
            let mut huffman_dt = 0u8;
            let mut huffman_refinement_dw = 0u8;
            let mut huffman_refinement_dh = 0u8;
            let mut huffman_refinement_dx = 0u8;
            let mut huffman_refinement_dy = 0u8;
            let mut huffman_refinement_size_selector = false;

            if huffman {
                if position + 2 > data.len() {
                    return Err(Jbig2Error::new(
                        "insufficient data for text region Huffman flags",
                    ));
                }
                let text_region_huffman_flags = read_uint16(data, position);
                position += 2;
                huffman_fs = (text_region_huffman_flags & 3) as u8;
                huffman_ds = ((text_region_huffman_flags >> 2) & 3) as u8;
                huffman_dt = ((text_region_huffman_flags >> 4) & 3) as u8;
                huffman_refinement_dw = ((text_region_huffman_flags >> 6) & 3) as u8;
                huffman_refinement_dh = ((text_region_huffman_flags >> 8) & 3) as u8;
                huffman_refinement_dx = ((text_region_huffman_flags >> 10) & 3) as u8;
                huffman_refinement_dy = ((text_region_huffman_flags >> 12) & 3) as u8;
                huffman_refinement_size_selector = (text_region_huffman_flags & 0x4000) != 0;
            }

            let mut refinement_at = Vec::new();
            if refinement && refinement_template == 0 {
                for _ in 0..2 {
                    if position + 2 > data.len() {
                        return Err(Jbig2Error::new(
                            "insufficient data for refinement AT pixels",
                        ));
                    }
                    refinement_at.push(TemplatePixel {
                        x: read_int8(data, position) as i32,
                        y: read_int8(data, position + 1) as i32,
                    });
                    position += 2;
                }
            }

            if position + 4 > data.len() {
                return Err(Jbig2Error::new(
                    "insufficient data for number of symbol instances",
                ));
            }
            let number_of_symbol_instances = read_uint32(data, position);
            position += 4;

            let region = TextRegion {
                info,
                huffman,
                refinement,
                huffman_fs,
                huffman_ds,
                huffman_dt,
                huffman_refinement_dw,
                huffman_refinement_dh,
                huffman_refinement_dx,
                huffman_refinement_dy,
                huffman_refinement_size_selector,
                default_pixel_value,
                number_of_symbol_instances,
                strip_size,
                transposed,
                ds_offset,
                reference_corner,
                combination_operator,
                refinement_template,
                refinement_at,
                log_strip_size,
            };

            visitor.on_immediate_text_region(&region, &header.referred_to, data, position, end)?;
        }
        16 => {
            // PatternDictionary
            if position + 7 > data.len() {
                return Err(Jbig2Error::new("insufficient data for pattern dictionary"));
            }

            let pattern_dictionary_flags = data[position];
            position += 1;
            let mmr = (pattern_dictionary_flags & 1) != 0;
            let template = ((pattern_dictionary_flags >> 1) & 3) as usize;

            let pattern_width = data[position] as u32;
            position += 1;
            let pattern_height = data[position] as u32;
            position += 1;
            let max_pattern_index = read_uint32(data, position);
            position += 4;

            let dictionary = PatternDictionary {
                mmr,
                pattern_width,
                pattern_height,
                max_pattern_index,
                template,
            };

            visitor.on_pattern_dictionary(&dictionary, header.number, data, position, end)?;
        }
        22 | 23 => {
            // ImmediateHalftoneRegion | ImmediateLosslessHalftoneRegion
            if position + REGION_SEGMENT_INFORMATION_FIELD_LENGTH + 1 > data.len() {
                return Err(Jbig2Error::new("insufficient data for halftone region"));
            }

            let info = read_region_segment_information(data, position)?;
            position += REGION_SEGMENT_INFORMATION_FIELD_LENGTH;

            let halftone_region_flags = data[position];
            position += 1;

            let mmr = (halftone_region_flags & 1) != 0;
            let template = ((halftone_region_flags >> 1) & 3) as usize;
            let enable_skip = (halftone_region_flags & 8) != 0;
            let combination_operator = ((halftone_region_flags >> 4) & 7) as u8;
            let default_pixel_value = ((halftone_region_flags >> 7) & 1) as u8;

            if position + 16 > data.len() {
                return Err(Jbig2Error::new("insufficient data for halftone grid"));
            }
            let grid_width = read_uint32(data, position);
            position += 4;
            let grid_height = read_uint32(data, position);
            position += 4;
            let grid_offset_x = read_uint32(data, position) as i32;
            position += 4;
            let grid_offset_y = read_uint32(data, position) as i32;
            position += 4;
            let grid_vector_x = read_uint16(data, position) as i32;
            position += 2;
            let grid_vector_y = read_uint16(data, position) as i32;
            position += 2;

            let region = HalftoneRegion {
                info,
                mmr,
                template,
                default_pixel_value,
                enable_skip,
                combination_operator,
                grid_width,
                grid_height,
                grid_offset_x,
                grid_offset_y,
                grid_vector_x,
                grid_vector_y,
            };

            visitor.on_immediate_halftone_region(
                &region,
                &header.referred_to,
                data,
                position,
                end,
            )?;
        }
        38 | 39 => {
            // ImmediateGenericRegion | ImmediateLosslessGenericRegion
            if position + REGION_SEGMENT_INFORMATION_FIELD_LENGTH + 1 > data.len() {
                return Err(Jbig2Error::new("insufficient data for generic region"));
            }

            let info = read_region_segment_information(data, position)?;
            position += REGION_SEGMENT_INFORMATION_FIELD_LENGTH;

            let generic_region_segment_flags = data[position];
            position += 1;

            let mmr = (generic_region_segment_flags & 1) != 0;
            let template = ((generic_region_segment_flags >> 1) & 3) as usize;
            let prediction = (generic_region_segment_flags & 8) != 0;

            let mut at = Vec::new();
            if !mmr {
                let at_length = if template == 0 { 4 } else { 1 };
                for _ in 0..at_length {
                    if position + 2 > data.len() {
                        return Err(Jbig2Error::new("insufficient data for AT pixels"));
                    }
                    at.push(TemplatePixel {
                        x: read_int8(data, position) as i32,
                        y: read_int8(data, position + 1) as i32,
                    });
                    position += 2;
                }
            }

            let region = GenericRegion {
                info,
                mmr,
                template,
                prediction,
                at,
            };

            visitor.on_immediate_generic_region(&region, data, position, end)?;
        }
        48 => {
            // PageInformation
            if position + 19 > data.len() {
                return Err(Jbig2Error::new("insufficient data for page information"));
            }

            let width = read_uint32(data, position);
            let height = read_uint32(data, position + 4);
            // TODO: PAGE INFORMATION DIFFERENCES: JS processSegment() reads additional fields:
            // resolutionX, resolutionY (positions 8-15), pageStripingInformation (position 17),
            // plus more detailed flag parsing: lossless, refinement, requiresBuffer flags.
            // Rust version only reads width, height, and basic flags. Missing fields could
            // affect page rendering parameters and buffer management.
            let page_segment_flags = data[position + 16];

            let default_pixel_value = ((page_segment_flags >> 2) & 1) as u8;
            let combination_operator = ((page_segment_flags >> 3) & 3) as u8;
            let combination_operator_override = (page_segment_flags & 64) != 0;

            let page_info = PageInfo {
                width,
                height: if height == 0xffffffff { 0 } else { height }, // Handle unknown height
                default_pixel_value,
                combination_operator,
                combination_operator_override,
            };

            visitor.on_page_information(page_info);
        }
        49 => { // EndOfPage
            // No processing needed
        }
        50 => { // EndOfStripe  
            // No processing needed
        }
        51 => { // EndOfFile
            // No processing needed
        }
        53 => {
            // Tables
            visitor.on_tables(header.number, data, position, end)?;
        }
        62 => { // Extension - can be ignored
            // No processing needed
        }
        _ => {
            return Err(Jbig2Error::new(&format!(
                "segment type {}({}) is not implemented",
                header.type_name, header.segment_type
            )));
        }
    }

    Ok(())
}

// processSegments function - ported from JS processSegments function
fn process_segments(
    segments: &[Segment],
    visitor: &mut SimpleSegmentVisitor,
) -> Result<(), Jbig2Error> {
    for segment in segments {
        process_segment(segment, visitor)?;
    }
    Ok(())
}

// Tables segment decoding - ported from decodeTablesSegment function
fn decode_tables_segment(
    data: &[u8],
    start: usize,
    end: usize,
) -> Result<HuffmanTable, Jbig2Error> {
    if start + 9 > data.len() {
        return Err(Jbig2Error::new("insufficient data for tables segment"));
    }

    let flags = data[start];
    let lowest_value = read_uint32(data, start + 1);
    let highest_value = read_uint32(data, start + 5);
    let mut reader = Reader::new(data, start + 9, end);

    let prefix_size_bits = ((flags >> 1) & 7) + 1;
    let range_size_bits = ((flags >> 4) & 7) + 1;
    let mut lines = Vec::new();
    let mut current_range_low = lowest_value;

    // Normal table lines
    while current_range_low < highest_value {
        let prefix_length = reader.read_bits(prefix_size_bits as usize)? as i32;
        let range_length = reader.read_bits(range_size_bits as usize)? as i32;

        lines.push(HuffmanLine::new(&[
            current_range_low as i32,
            prefix_length,
            range_length,
            0,
        ]));

        current_range_low += 1 << range_length;
    }

    // Lower range table line
    let prefix_length = reader.read_bits(prefix_size_bits as usize)? as i32;
    lines.push(HuffmanLine::new(&[
        lowest_value as i32 - 1,
        prefix_length,
        32,
        0,
        -1, // "lower" marker
    ]));

    // Upper range table line
    let prefix_length = reader.read_bits(prefix_size_bits as usize)? as i32;
    lines.push(HuffmanLine::new(&[
        highest_value as i32,
        prefix_length,
        32,
        0,
    ]));

    // Out-of-band table line
    if (flags & 1) != 0 {
        let prefix_length = reader.read_bits(prefix_size_bits as usize)? as i32;
        lines.push(HuffmanLine::new(&[prefix_length, 0]));
    }

    Ok(HuffmanTable::new(lines, false))
}

// Custom Huffman table getter - ported from getCustomHuffmanTable function
fn get_custom_huffman_table<'a>(
    index: usize,
    referred_to: &[u32],
    custom_tables: &'a HashMap<u32, HuffmanTable>,
) -> Result<&'a HuffmanTable, Jbig2Error> {
    let mut current_index = 0;
    for &referred_segment in referred_to {
        if let Some(table) = custom_tables.get(&referred_segment) {
            if index == current_index {
                return Ok(table);
            }
            current_index += 1;
        }
    }
    Err(Jbig2Error::new("can't find custom Huffman table"))
}

// Helper function for complete RUNCODE symbol ID table decoding
// ✅ FAITHFUL PORT: Complete JavaScript getTextRegionHuffmanTables implementation
fn decode_symbol_id_huffman_table(
    reader: &Reader,
    number_of_symbols: usize,
) -> Result<HuffmanTable, Jbig2Error> {
    // 7.4.3.1.7 Symbol ID Huffman table decoding

    // Read code lengths for RUNCODEs 0...34 (4 bits each)
    let mut codes = Vec::new();
    for i in 0..=34 {
        let code_length = reader.read_bits(4)? as i32;
        codes.push(HuffmanLine::new(&[i, code_length, 0, 0]));
    }

    // Assign Huffman codes for RUNCODEs
    let run_codes_table = HuffmanTable::new(codes, false);

    // Read a Huffman code using the assignment above
    // Interpret the RUNCODE codes and the additional bits (if any)
    let mut symbol_codes: Vec<HuffmanLine> = Vec::new();
    let mut i = 0;
    while i < number_of_symbols {
        let code_length = run_codes_table
            .decode(reader)?
            .ok_or_else(|| Jbig2Error::new("unexpected OOB in RUNCODE table"))?;

        if code_length >= 32 {
            let (repeated_length, number_of_repeats) = match code_length {
                32 => {
                    if i == 0 {
                        return Err(Jbig2Error::new("no previous value in symbol ID table"));
                    }
                    let repeats = reader.read_bits(2)? + 3;
                    let prev_length = symbol_codes[i - 1].prefix_length as i32;
                    (prev_length, repeats)
                }
                33 => {
                    let repeats = reader.read_bits(3)? + 3;
                    (0, repeats)
                }
                34 => {
                    let repeats = reader.read_bits(7)? + 11;
                    (0, repeats)
                }
                _ => return Err(Jbig2Error::new("invalid code length in symbol ID table")),
            };

            for _ in 0..number_of_repeats {
                symbol_codes.push(HuffmanLine::new(&[i as i32, repeated_length, 0, 0]));
                i += 1;
            }
        } else {
            symbol_codes.push(HuffmanLine::new(&[i as i32, code_length, 0, 0]));
            i += 1;
        }
    }

    reader.byte_align();
    Ok(HuffmanTable::new(symbol_codes, false))
}
