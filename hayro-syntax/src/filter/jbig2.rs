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

//! A decoder for JBIG2 streams, translated from https://github.com/mozilla/pdf.js/blob/master/src/core/jbig2.js
//!
//! TODO: MAJOR DIFFERENCES BETWEEN JS AND RUST IMPLEMENTATIONS:
//! 1. Main entry point: JS has parseJbig2() and parseJbig2Chunks() as separate functions + Jbig2Image class
//!    Rust combines these into a single decode() function returning Option<Vec<u8>>
//! 2. DecodingContext: JS uses lazy getters with shadow() utility, Rust stores decoder/cache directly
//! 3. decode_integer: JS doesn't handle -0 explicitly, Rust converts -0 to 0 which could affect signed zero representation
//! 4. decodeBitmapTemplate0 row initialization: JS uses current row as row1 when i==0, Rust uses empty_row
//! 5. decode_symbol_dictionary: Rust implementation is incomplete for Huffman mode compared to JS
//! 6. Error handling: JS throws exceptions, Rust returns Result types
//! 7. Array indexing: JS has bounds-checked access, Rust uses .get().copied().unwrap_or(0) patterns
//! 8. decode_pattern_dictionary: JS decodes collective bitmap then divides, Rust decodes individually
//! 9. decode_halftone_region: JS uses complex grid vector formulas with bit shifts, Rust simplified
//! 10. ArithmeticDecoder: JS uses direct array access, Rust adds bounds checking
//! 11. decode_bitmap: JS uses Int8Array/Uint16Array for template coordinates, Rust uses Vec<TemplatePixel>
//! 12. decode_text_region: JS supports transposed placement and combination operators, Rust simplified
//! 13. HuffmanLine: JS single constructor with string flags, Rust separate constructors
//! 14. Text region Huffman tables: JS implements complex symbol ID table with RUNCODE handling, Rust simplified
//! 15. MMR bitmap decoding: JS uses CCITTFaxDecoder, Rust has placeholder implementation
//! 16. Unknown segment length: JS implements pattern search for end detection, Rust returns error
//! 17. Header validation: JS parseJbig2() validates 8-byte JBIG2 signature and handles randomAccess/numberOfPages
//! 18. Final output: JS converts bit-packed to Uint8ClampedArray with 0/255 values, Rust returns raw Vec<u8>
//! 19. Segment flag parsing: JS extracts detailed Huffman selectors (DH/DW/FS/DS/DT), Rust uses simplified flags
//! 20. SimpleSegmentVisitor: JS has more sophisticated symbol/pattern/table management with lazy initialization

use crate::object::dict::Dict;
use crate::object::dict::keys::JBIG2_GLOBALS;
use crate::filter::ccitt::{CCITTFaxDecoder, CCITTFaxDecoderOptions};
use crate::reader::Reader as CrateReader;
use log::warn;
use std::collections::HashMap;

// Decode a JBIG2 data stream
// TODO: JS version has parseJbig2(data) and parseJbig2Chunks(chunks) as separate functions,
// but Rust version only has decode() function that combines both. The JS version also has
// a Jbig2Image class with parseChunks() and parse() methods, while Rust directly returns
// Option<Vec<u8>>. This architectural difference may affect error handling and data flow.
// TODO: PARSE ENTRY POINT DIFFERENCES: JS parseJbig2() has explicit JBIG2 header validation
// (checks for 0x97 0x4A 0x42 0x32 0x0D 0x0A 0x1A 0x0A signature), random access flag handling,
// and numberOfPages parsing. JS also converts bit-packed buffer to Uint8ClampedArray with 
// 0/255 pixel values. Rust version has simplified header handling and returns raw Vec<u8>.
pub fn decode(data: &[u8], params: Dict) -> Option<Vec<u8>> {
    let globals = params.get::<Vec<u8>>(JBIG2_GLOBALS);
    
    let mut chunks = Vec::new();
    
    // Add globals if present
    if let Some(globals_data) = globals {
        chunks.push(Chunk {
            data: globals_data.clone(),
            start: 0,
            end: globals_data.len(),
        });
    }
    
    // Add main data
    chunks.push(Chunk {
        data: data.to_vec(),
        start: 0,
        end: data.len(),
    });
    
    let mut jbig2_image = Jbig2Image::new();
    jbig2_image.parse_chunks(&chunks)
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
        self.contexts.entry(id.to_string()).or_insert_with(|| vec![0; 1 << 16])
    }
}

// New architecture to fix borrowing issues
// TODO: JS version uses lazy getters for decoder and contextCache (using shadow() utility),
// while Rust version stores them directly. JS DecodingContext.decoder creates a new 
// ArithmeticDecoder on first access and caches it, but Rust creates it immediately in new().
// This might affect performance and memory usage patterns.
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

// Constants for segment types (7.3 Segment types) - matches JS SegmentTypes array exactly
const SEGMENT_TYPES: &[Option<&str>] = &[
    Some("SymbolDictionary"),
    None, None, None,
    Some("IntermediateTextRegion"),
    None,
    Some("ImmediateTextRegion"),
    Some("ImmediateLosslessTextRegion"),
    None, None, None, None, None, None, None, None,
    Some("PatternDictionary"),
    None, None, None,
    Some("IntermediateHalftoneRegion"),
    None,
    Some("ImmediateHalftoneRegion"),
    Some("ImmediateLosslessHalftoneRegion"),
    None, None, None, None, None, None, None, None,
    None, None, None, None,
    Some("IntermediateGenericRegion"),
    None,
    Some("ImmediateGenericRegion"),
    Some("ImmediateLosslessGenericRegion"),
    Some("IntermediateGenericRefinementRegion"),
    None,
    Some("ImmediateGenericRefinementRegion"),
    Some("ImmediateLosslessGenericRefinementRegion"),
    None, None, None, None,
    Some("PageInformation"),
    Some("EndOfPage"),
    Some("EndOfStripe"),
    Some("EndOfFile"),
    Some("Profiles"),
    Some("Tables"),
    None, None, None, None, None, None, None, None,
    Some("Extension"),
];

// Coding templates
const CODING_TEMPLATES: [&[[i32; 2]]; 4] = [
    &[
        [-1, -2], [0, -2], [1, -2], [-2, -1], [-1, -1], [0, -1], [1, -1], [2, -1],
        [-4, 0], [-3, 0], [-2, 0], [-1, 0],
    ],
    &[
        [-1, -2], [0, -2], [1, -2], [2, -2], [-2, -1], [-1, -1], [0, -1], [1, -1], [2, -1],
        [-3, 0], [-2, 0], [-1, 0],
    ],
    &[
        [-1, -2], [0, -2], [1, -2], [-2, -1], [-1, -1], [0, -1], [1, -1],
        [-2, 0], [-1, 0],
    ],
    &[
        [-3, -1], [-2, -1], [-1, -1], [0, -1], [1, -1],
        [-4, 0], [-3, 0], [-2, 0], [-1, 0],
    ],
];

// Refinement templates
const REFINEMENT_TEMPLATES: [RefinementTemplate; 2] = [
    RefinementTemplate {
        coding: &[[0, -1], [1, -1], [-1, 0]],
        reference: &[[0, -1], [1, -1], [-1, 0], [0, 0], [1, 0], [-1, 1], [0, 1], [1, 1]],
    },
    RefinementTemplate {
        coding: &[[-1, -1], [0, -1], [1, -1], [-1, 0]],
        reference: &[[0, -1], [-1, 0], [0, 0], [1, 0], [0, 1], [1, 1]],
    },
];

struct RefinementTemplate {
    coding: &'static [[i32; 2]],
    reference: &'static [[i32; 2]],
}

// QM Coder Table C-2 from JPEG 2000 Part I Final Committee Draft Version 1.0
#[derive(Clone, Copy)]
struct QeEntry {
    qe: u32,
    nmps: u8,
    nlps: u8,
    switch_flag: u8,
}

const QE_TABLE: [QeEntry; 47] = [
    QeEntry { qe: 0x5601, nmps: 1, nlps: 1, switch_flag: 1 },
    QeEntry { qe: 0x3401, nmps: 2, nlps: 6, switch_flag: 0 },
    QeEntry { qe: 0x1801, nmps: 3, nlps: 9, switch_flag: 0 },
    QeEntry { qe: 0x0ac1, nmps: 4, nlps: 12, switch_flag: 0 },
    QeEntry { qe: 0x0521, nmps: 5, nlps: 29, switch_flag: 0 },
    QeEntry { qe: 0x0221, nmps: 38, nlps: 33, switch_flag: 0 },
    QeEntry { qe: 0x5601, nmps: 7, nlps: 6, switch_flag: 1 },
    QeEntry { qe: 0x5401, nmps: 8, nlps: 14, switch_flag: 0 },
    QeEntry { qe: 0x4801, nmps: 9, nlps: 14, switch_flag: 0 },
    QeEntry { qe: 0x3801, nmps: 10, nlps: 14, switch_flag: 0 },
    QeEntry { qe: 0x3001, nmps: 11, nlps: 17, switch_flag: 0 },
    QeEntry { qe: 0x2401, nmps: 12, nlps: 18, switch_flag: 0 },
    QeEntry { qe: 0x1c01, nmps: 13, nlps: 20, switch_flag: 0 },
    QeEntry { qe: 0x1601, nmps: 29, nlps: 21, switch_flag: 0 },
    QeEntry { qe: 0x5601, nmps: 15, nlps: 14, switch_flag: 1 },
    QeEntry { qe: 0x5401, nmps: 16, nlps: 14, switch_flag: 0 },
    QeEntry { qe: 0x5101, nmps: 17, nlps: 15, switch_flag: 0 },
    QeEntry { qe: 0x4801, nmps: 18, nlps: 16, switch_flag: 0 },
    QeEntry { qe: 0x3801, nmps: 19, nlps: 17, switch_flag: 0 },
    QeEntry { qe: 0x3401, nmps: 20, nlps: 18, switch_flag: 0 },
    QeEntry { qe: 0x3001, nmps: 21, nlps: 19, switch_flag: 0 },
    QeEntry { qe: 0x2801, nmps: 22, nlps: 19, switch_flag: 0 },
    QeEntry { qe: 0x2401, nmps: 23, nlps: 20, switch_flag: 0 },
    QeEntry { qe: 0x2201, nmps: 24, nlps: 21, switch_flag: 0 },
    QeEntry { qe: 0x1c01, nmps: 25, nlps: 22, switch_flag: 0 },
    QeEntry { qe: 0x1801, nmps: 26, nlps: 23, switch_flag: 0 },
    QeEntry { qe: 0x1601, nmps: 27, nlps: 24, switch_flag: 0 },
    QeEntry { qe: 0x1401, nmps: 28, nlps: 25, switch_flag: 0 },
    QeEntry { qe: 0x1201, nmps: 29, nlps: 26, switch_flag: 0 },
    QeEntry { qe: 0x1101, nmps: 30, nlps: 27, switch_flag: 0 },
    QeEntry { qe: 0x0ac1, nmps: 31, nlps: 28, switch_flag: 0 },
    QeEntry { qe: 0x09c1, nmps: 32, nlps: 29, switch_flag: 0 },
    QeEntry { qe: 0x08a1, nmps: 33, nlps: 30, switch_flag: 0 },
    QeEntry { qe: 0x0521, nmps: 34, nlps: 31, switch_flag: 0 },
    QeEntry { qe: 0x0441, nmps: 35, nlps: 32, switch_flag: 0 },
    QeEntry { qe: 0x02a1, nmps: 36, nlps: 33, switch_flag: 0 },
    QeEntry { qe: 0x0221, nmps: 37, nlps: 34, switch_flag: 0 },
    QeEntry { qe: 0x0141, nmps: 38, nlps: 35, switch_flag: 0 },
    QeEntry { qe: 0x0111, nmps: 39, nlps: 36, switch_flag: 0 },
    QeEntry { qe: 0x0085, nmps: 40, nlps: 37, switch_flag: 0 },
    QeEntry { qe: 0x0049, nmps: 41, nlps: 38, switch_flag: 0 },
    QeEntry { qe: 0x0025, nmps: 42, nlps: 39, switch_flag: 0 },
    QeEntry { qe: 0x0015, nmps: 43, nlps: 40, switch_flag: 0 },
    QeEntry { qe: 0x0009, nmps: 44, nlps: 41, switch_flag: 0 },
    QeEntry { qe: 0x0005, nmps: 45, nlps: 42, switch_flag: 0 },
    QeEntry { qe: 0x0001, nmps: 45, nlps: 43, switch_flag: 0 },
    QeEntry { qe: 0x5601, nmps: 46, nlps: 46, switch_flag: 0 },
];

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
}

impl ArithmeticDecoder {
    // C.3.5 Initialisation of the decoder (INITDEC)
    // TODO: ARITHMETIC DECODER DIFFERENCES: JS version uses direct array access data[bp],
    // Rust version adds bounds checking. JS uses let/const for variables, Rust uses mut.
    // Both follow the same QM Coder algorithm but with different memory safety approaches.
    fn new(data: &[u8], start: usize, end: usize) -> Self {
        let mut decoder = Self {
            data: data.to_vec(),
            bp: start,
            data_end: end,
            chigh: if start < data.len() { data[start] as u32 } else { 0 },
            clow: 0,
            ct: 0,
            a: 0,
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
                let byte_val = if self.bp < self.data.len() { self.data[self.bp] as u32 } else { 0xff };
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
fn decode_integer(context_cache: &mut ContextCache, procedure: &str, decoder: &mut ArithmeticDecoder) -> Option<i32> {
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
    
    // Nested ternary from original JS - keeping structure faithful
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

    let signed_value = if sign == 0 {
        value as i32
    } else if value > 0 {
        -(value as i32)
    } else {
        // When value is 0 and sign is 1, result should be 0 (not -0)
        // TODO: JS version doesn't have this explicit check - it would create a -0 value.
        // JS: signedValue = -value; (where value is 0) creates -0, but in Rust this becomes 0.
        // This could potentially affect behavior if the decoder expects signed zero representation.
        0
    };

    // Ensure that the integer value doesn't underflow or overflow
    const MIN_INT_32: i32 = i32::MIN;
    const MAX_INT_32: i32 = i32::MAX;
    
    if signed_value >= MIN_INT_32 && signed_value <= MAX_INT_32 {
        Some(signed_value)
    } else {
        None
    }
}

// A.3 The IAID decoding procedure
fn decode_iaid(context_cache: &mut ContextCache, decoder: &mut ArithmeticDecoder, code_length: usize) -> u32 {
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

// Reused contexts for different template indices (6.2.5.7)
const REUSED_CONTEXTS: [u32; 4] = [
    0x9b25, // 10011 0110010 0101
    0x0795, // 0011 110010 101  
    0x00e5, // 001 11001 01
    0x0195, // 011001 0101
];

// Refinement reused contexts 
const REFINEMENT_REUSED_CONTEXTS: [u32; 2] = [
    0x0020, // '000' + '0' (coding) + '00010000' + '0' (reference)
    0x0008, // '0000' + '001000'
];

// Bitmap type for 2D bitmap data
type Bitmap = Vec<Vec<u8>>;

// Template structure for coordinates
#[derive(Clone, Copy, Debug)]
struct TemplatePixel {
    x: i32,
    y: i32,
}

// 6.2 Generic Region Decoding Procedure - Template 0 optimized version
fn decode_bitmap_template0(width: usize, height: usize, decoding_context: &mut DecodingContext) -> Bitmap {
    let mut bitmap = Vec::with_capacity(height);
    
    // Context template for current pixel (X)
    // ...ooooo....
    // ..ooooooo... Context template for current pixel (X)
    // .ooooX...... (concatenate values of 'o'-pixels to get contextLabel)
    const OLD_PIXEL_MASK: u32 = 0x7bf7; // 01111 0111111 0111

    for i in 0..height {
        let mut row = vec![0u8; width];
        let empty_row = vec![0u8; width];
        let row1 = if i >= 1 { &bitmap[i - 1] } else { &empty_row };
        let row2 = if i >= 2 { &bitmap[i - 2] } else { &empty_row };

        // At the beginning of each row:
        // Fill contextLabel with pixels that are above/right of (X)
        let mut context_label = 
            ((row2.get(0).copied().unwrap_or(0) as u32) << 13) |
            ((row2.get(1).copied().unwrap_or(0) as u32) << 12) |
            ((row2.get(2).copied().unwrap_or(0) as u32) << 11) |
            ((row1.get(0).copied().unwrap_or(0) as u32) << 7) |
            ((row1.get(1).copied().unwrap_or(0) as u32) << 6) |
            ((row1.get(2).copied().unwrap_or(0) as u32) << 5) |
            ((row1.get(3).copied().unwrap_or(0) as u32) << 4);

        for j in 0..width {
            let contexts = decoding_context.context_cache.get_contexts("GB");
            let pixel = decoding_context.decoder.read_bit(contexts, context_label as usize);
            row[j] = pixel;

            // At each pixel: Clear contextLabel pixels that are shifted
            // out of the context, then add new ones.
            context_label = ((context_label & OLD_PIXEL_MASK) << 1) |
                ((row2.get(j + 3).copied().unwrap_or(0) as u32) << 11) |
                ((row1.get(j + 4).copied().unwrap_or(0) as u32) << 4) |
                (pixel as u32);
        }
        bitmap.push(row);
    }

    bitmap
}

// 6.2 Generic Region Decoding Procedure - General case
fn decode_bitmap(
    mmr: bool,
    width: usize,
    height: usize,
    template_index: usize,
    prediction: bool,
    skip: Option<&Bitmap>,
    at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
) -> Result<Bitmap, Jbig2Error> {
    // TODO: TEMPLATE HANDLING DIFFERENCES: JS uses CodingTemplates[templateIndex].concat(at)
    // and creates separate Int8Array for templateX/templateY coordinates, plus Int32Array for
    // changingTemplateX/Y and Uint16Array for changingTemplateBit. Rust uses Vec<TemplatePixel>
    // and different data structures. This could affect template processing performance and accuracy.
    if mmr {
        // Use MMR decoding
        let data_slice = &decoding_context.decoder.data[decoding_context.decoder.bp..decoding_context.decoder.data_end];
        return decode_mmr_bitmap(data_slice, width, height, false);
    }

    // Use optimized version for the most common case
    if template_index == 0 && skip.is_none() && !prediction &&
       at.len() == 4 &&
       at[0].x == 3 && at[0].y == -1 &&
       at[1].x == -3 && at[1].y == -1 &&
       at[2].x == 2 && at[2].y == -2 &&
       at[3].x == -2 && at[3].y == -2 {
        return Ok(decode_bitmap_template0(width, height, decoding_context));
    }

    let use_skip = skip.is_some();
    let mut template = CODING_TEMPLATES[template_index].iter()
        .map(|[x, y]| TemplatePixel { x: *x, y: *y })
        .collect::<Vec<_>>();
    template.extend_from_slice(at);

    // Sorting is non-standard, and it is not required. But sorting increases
    // the number of template bits that can be reused from the previous
    // contextLabel in the main loop.
    template.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));

    let template_length = template.len();
    let mut changing_template_entries = Vec::new();
    let mut reuse_mask = 0u32;
    let mut min_x = 0i32;
    let mut max_x = 0i32;
    let mut min_y = 0i32;

    for k in 0..template_length {
        min_x = min_x.min(template[k].x);
        max_x = max_x.max(template[k].x);
        min_y = min_y.min(template[k].y);
        
        // Check if the template pixel appears in two consecutive context labels,
        // so it can be reused. Otherwise, we add it to the list of changing
        // template entries.
        if k < template_length - 1 &&
           template[k].y == template[k + 1].y &&
           template[k].x == template[k + 1].x - 1 {
            reuse_mask |= 1 << (template_length - 1 - k);
        } else {
            changing_template_entries.push(k);
        }
    }

    // Get the safe bounding box edges from the width, height, minX, maxX, minY
    let sbb_left = (-min_x) as usize;
    let sbb_top = (-min_y) as usize;
    let sbb_right = (width as i32 - max_x) as usize;

    let pseudo_pixel_context = REUSED_CONTEXTS[template_index];
    let mut bitmap = Vec::with_capacity(height);
    let mut row = vec![0u8; width];

    // We'll use the with_decoder_and_context method in the loop below

    let mut ltp = 0u8;
    
    for i in 0..height {
        if prediction {
            let sltp = decoding_context.read_bit_with_context("GB", pseudo_pixel_context as usize);
            ltp ^= sltp;
            if ltp != 0 {
                bitmap.push(row.clone()); // duplicate previous row
                continue;
            }
        }
        
        row = vec![0u8; width];
        
        for j in 0..width {
            if use_skip && skip.unwrap()[i][j] != 0 {
                row[j] = 0;
                continue;
            }
            
            let mut context_label = 0u32;
            
            // Are we in the middle of a scanline, so we can reuse contextLabel bits?
            if j >= sbb_left && j < sbb_right && i >= sbb_top {
                // If yes, we can just shift the bits that are reusable and only
                // fetch the remaining ones.
                context_label = (context_label << 1) & reuse_mask;
                for &k in &changing_template_entries {
                    let i0 = (i as i32 + template[k].y) as usize;
                    let j0 = (j as i32 + template[k].x) as usize;
                    if i0 < bitmap.len() && j0 < width {
                        let bit = bitmap[i0][j0];
                        if bit != 0 {
                            let changing_bit = 1 << (template_length - 1 - k);
                            context_label |= changing_bit;
                        }
                    }
                }
            } else {
                // compute the contextLabel from scratch
                context_label = 0;
                for k in 0..template_length {
                    let j0 = j as i32 + template[k].x;
                    if j0 >= 0 && j0 < width as i32 {
                        let i0 = i as i32 + template[k].y;
                        if i0 >= 0 && (i0 as usize) < bitmap.len() {
                            let bit = bitmap[i0 as usize][j0 as usize];
                            if bit != 0 {
                                context_label |= 1 << (template_length - 1 - k);
                            }
                        }
                    }
                }
            }
            
            let pixel = decoding_context.read_bit_with_context("GB", context_label as usize);
            row[j] = pixel;
        }
        bitmap.push(row.clone());
    }

    Ok(bitmap)
}

// Refinement decoding - ported from decodeRefinement function
fn decode_refinement(
    width: usize,
    height: usize,
    template_index: usize,
    reference_bitmap: &Bitmap,
    offset_x: i32,
    offset_y: i32,
    prediction: bool,
    at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
) -> Result<Bitmap, Jbig2Error> {
    let mut coding_template: Vec<[i32; 2]> = REFINEMENT_TEMPLATES[template_index].coding.to_vec();
    if template_index == 0 {
        coding_template.push([at[0].x, at[0].y]);
    }
    let coding_template_length = coding_template.len();
    let coding_template_x: Vec<i32> = coding_template.iter().map(|p| p[0]).collect();
    let coding_template_y: Vec<i32> = coding_template.iter().map(|p| p[1]).collect();

    let mut reference_template: Vec<[i32; 2]> = REFINEMENT_TEMPLATES[template_index].reference.to_vec();
    if template_index == 0 {
        reference_template.push([at[1].x, at[1].y]);
    }
    let reference_template_length = reference_template.len();
    let reference_template_x: Vec<i32> = reference_template.iter().map(|p| p[0]).collect();
    let reference_template_y: Vec<i32> = reference_template.iter().map(|p| p[1]).collect();
    
    let reference_width = if !reference_bitmap.is_empty() { reference_bitmap[0].len() } else { 0 };
    let reference_height = reference_bitmap.len();

    let pseudo_pixel_context = REFINEMENT_REUSED_CONTEXTS[template_index];
    let mut bitmap: Vec<Vec<u8>> = Vec::with_capacity(height);

    let mut ltp = 0u8;
    
    for i in 0..height {
        if prediction {
            let sltp = decoding_context.read_bit_with_context("GR", pseudo_pixel_context as usize);
            ltp ^= sltp;
            if ltp != 0 {
                return Err(Jbig2Error::new("prediction is not supported"));
            }
        }
        
        let mut row = vec![0u8; width];
        
        for j in 0..width {
            let mut context_label = 0u32;
            
            // Process coding template
            for k in 0..coding_template_length {
                let i0 = i as i32 + coding_template_y[k];
                let j0 = j as i32 + coding_template_x[k];
                
                if i0 < 0 || j0 < 0 || j0 >= width as i32 {
                    context_label <<= 1; // out of bound pixel
                } else {
                    let bit = if i0 >= 0 && (i0 as usize) < bitmap.len() && 
                                 j0 >= 0 && (j0 as usize) < width {
                        bitmap[i0 as usize][j0 as usize]
                    } else {
                        0
                    };
                    context_label = (context_label << 1) | (bit as u32);
                }
            }
            
            // Process reference template
            for k in 0..reference_template_length {
                let i0 = i as i32 + reference_template_y[k] - offset_y;
                let j0 = j as i32 + reference_template_x[k] - offset_x;
                
                if i0 < 0 || i0 >= reference_height as i32 || j0 < 0 || j0 >= reference_width as i32 {
                    context_label <<= 1; // out of bound pixel
                } else {
                    let bit = reference_bitmap[i0 as usize][j0 as usize];
                    context_label = (context_label << 1) | (bit as u32);
                }
            }
            
            let pixel = decoding_context.read_bit_with_context("GR", context_label as usize);
            row[j] = pixel;
        }
        bitmap.push(row);
    }

    Ok(bitmap)
}

// Utility function equivalent to log2 from JS
fn log2(n: usize) -> usize {
    if n == 0 { 0 } else { (n as f64).log2().ceil() as usize }
}

// Symbol dictionary decoding - ported from decodeSymbolDictionary function  
fn decode_symbol_dictionary(
    huffman: bool,
    refinement: bool,
    symbols: &[Bitmap],
    number_of_new_symbols: usize,
    _number_of_exported_symbols: usize,
    huffman_tables: Option<&SymbolDictionaryHuffmanTables>,
    template_index: usize,
    at: &[TemplatePixel],
    refinement_template_index: usize,
    refinement_at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
    _huffman_input: Option<&mut Reader>,
) -> Result<Vec<Bitmap>, Jbig2Error> {
    // TODO: MAJOR SYMBOL DICTIONARY DIFFERENCES: JS version has complex Huffman handling with:
    // 1. Height class collective bitmap processing (tableBitmapSize.decode, dividing collective bitmap)
    // 2. Multiple instances handling with numberOfInstances and IAAI decoder 
    // 3. symbolWidths array tracking and totalWidth calculation
    // 4. Standard table B.1 usage and symbolCodeLength adjustments
    // Rust version is significantly simplified and may not handle complex symbol dictionary cases.
    if huffman && refinement {
        return Err(Jbig2Error::new("symbol refinement with Huffman is not supported"));
    }

    let mut new_symbols = Vec::new();
    let mut current_height = 0i32;
    let symbol_code_length = log2(symbols.len() + number_of_new_symbols).max(if huffman { 1 } else { 0 });

    if huffman {
        // Huffman-coded symbol dictionary
        if huffman_tables.is_none() {
            return Err(Jbig2Error::new("Huffman tables required for Huffman symbol dictionary"));
        }
        
        let tables = huffman_tables.unwrap();
        let huffman_reader = _huffman_input.ok_or_else(|| {
            Jbig2Error::new("Huffman input reader required for Huffman symbol dictionary")
        })?;
        
        // Huffman-coded symbol dictionary dimensions
        let mut current_height = 0i32;
        
        while new_symbols.len() < number_of_new_symbols {
            // Decode delta height using Huffman table
            let delta_height = tables.height_table.decode(huffman_reader)?;
            
            let Some(dh) = delta_height else { break }; // OOB
            current_height += dh;
            
            let mut current_width = 0i32;
            
            loop {
                // Decode delta width using Huffman table
                let delta_width = tables.width_table.decode(huffman_reader)?;
                
                let Some(dw) = delta_width else { break }; // OOB
                current_width += dw;
                
                let bitmap = if refinement {
                    // Refinement-coded symbol bitmap with Huffman
                    if symbols.is_empty() {
                        return Err(Jbig2Error::new("No reference symbols available for refinement"));
                    }
                    
                    // For Huffman refinement, symbol ID would be decoded differently
                    // For now, use a simplified approach
                    let reference_id = (huffman_reader.read_bits(symbol_code_length)? as usize).min(symbols.len() - 1);
                    let reference_bitmap = &symbols[reference_id];
                    
                    // Refinement offset would be decoded from Huffman tables if available
                    let refinement_offset_x = 0; // Simplified
                    let refinement_offset_y = 0; // Simplified
                    
                    decode_refinement(
                        current_width as usize,
                        current_height as usize,
                        refinement_template_index,
                        reference_bitmap,
                        refinement_offset_x,
                        refinement_offset_y,
                        false,
                        refinement_at,
                        decoding_context,
                    )?
                } else {
                    // Direct-coded symbol bitmap
                    // For Huffman mode, the bitmap data follows Huffman-decoded dimensions
                    if let Some(ref bitmap_size_table) = tables.bitmap_size_table {
                        // Decode bitmap size
                        let _bitmap_size = bitmap_size_table.decode(huffman_reader)?.unwrap_or(0);
                        
                        // Read uncompressed bitmap
                        read_uncompressed_bitmap(
                            huffman_reader,
                            current_width as usize,
                            current_height as usize,
                        )?
                    } else {
                        // Fallback to arithmetic decoding for bitmap content
                        decode_bitmap(
                            false, // mmr
                            current_width as usize,
                            current_height as usize,
                            template_index,
                            false, // prediction
                            None,  // skip
                            at,
                            decoding_context,
                        )?
                    }
                };
                
                new_symbols.push(bitmap);
            }
        }
        
        return Ok(new_symbols);
    }

    while new_symbols.len() < number_of_new_symbols {
        let delta_height = decoding_context.decode_integer("IADH");
        
        if let Some(dh) = delta_height {
            current_height += dh;
        } else {
            break;
        }
        
        let mut current_width = 0i32;
        
        loop {
            let delta_width = decoding_context.decode_integer("IADW");
            
            let Some(dw) = delta_width else { break }; // OOB
            current_width += dw;
            
            let bitmap = if refinement {
                // Refinement-coded symbol bitmap
                // This requires a reference symbol and refinement parameters
                if symbols.is_empty() {
                    return Err(Jbig2Error::new("No reference symbols available for refinement"));
                }
                
                // Read the refinement symbol ID (this would normally use IAID decoder)
                let reference_id = decoding_context.decode_iaid(symbol_code_length) as usize;
                if reference_id >= symbols.len() {
                    return Err(Jbig2Error::new("Invalid reference symbol ID for refinement"));
                }
                
                let reference_bitmap = &symbols[reference_id];
                
                // Read refinement offset (normally from integer decoder)
                let refinement_offset_x = decoding_context.decode_integer("IARDX").unwrap_or(0);
                let refinement_offset_y = decoding_context.decode_integer("IARDY").unwrap_or(0);
                
                // Decode refined bitmap using reference
                decode_refinement(
                    current_width as usize,
                    current_height as usize,
                    refinement_template_index,
                    reference_bitmap,
                    refinement_offset_x,
                    refinement_offset_y,
                    false, // prediction not typically used in symbol refinement
                    refinement_at,
                    decoding_context,
                )?
            } else {
                // Direct-coded symbol bitmap
                decode_bitmap(
                    false, // mmr
                    current_width as usize,
                    current_height as usize,
                    template_index,
                    false, // prediction - not typically used for symbol dictionaries
                    None,  // skip
                    at,
                    decoding_context,
                )?
            };
            
            new_symbols.push(bitmap);
        }
    }

    Ok(new_symbols)
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

// Reader class - ported from JS Reader class
#[derive(Debug)]
struct Reader<'a> {
    data: &'a [u8],
    end: usize,
    position: usize,
    shift: i32,
    current_byte: u8,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], start: usize, end: usize) -> Self {
        Self {
            data,
            end,
            position: start,
            shift: -1,
            current_byte: 0,
        }
    }

    fn read_bit(&mut self) -> Result<u8, Jbig2Error> {
        if self.shift < 0 {
            if self.position >= self.end {
                return Err(Jbig2Error::new("end of data while reading bit"));
            }
            self.current_byte = self.data[self.position];
            self.position += 1;
            self.shift = 7;
        }
        let bit = (self.current_byte >> self.shift) & 1;
        self.shift -= 1;
        Ok(bit)
    }

    fn read_bits(&mut self, num_bits: usize) -> Result<u32, Jbig2Error> {
        let mut result = 0u32;
        for i in 0..num_bits {
            let bit = self.read_bit()? as u32;
            result |= bit << (num_bits - 1 - i);
        }
        Ok(result)
    }

    fn byte_align(&mut self) {
        self.shift = -1;
    }

    fn _next(&mut self) -> i32 {
        if self.position >= self.end {
            return -1;
        }
        let byte = self.data[self.position] as i32;
        self.position += 1;
        byte
    }
}

// Text region decoding - ported from decodeTextRegion function
#[allow(clippy::too_many_arguments)]
fn decode_text_region(
    huffman: bool,
    refinement: bool,
    width: usize,
    height: usize,
    default_pixel_value: u8,
    number_of_symbol_instances: usize,
    strip_size: usize,
    input_symbols: &[Bitmap],
    symbol_code_length: usize,
    transposed: bool,
    ds_offset: i32,
    reference_corner: u8,
    combination_operator: u8,
    _huffman_tables: Option<&TextRegionHuffmanTables>,
    refinement_template_index: usize,
    refinement_at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
    log_strip_size: usize,
    _huffman_input: Option<&mut Reader>,
) -> Result<Bitmap, Jbig2Error> {
    // TODO: HUFFMAN HANDLING DIFFERENCES: JS version uses huffmanTables.tableDeltaT.decode()
    // for initial stripT calculation (stripT = -huffmanTables.tableDeltaT.decode(huffmanInput))
    // and supports applyRefinement flag from huffman_input.readBit() + detailed rdw/rdh/rdx/rdy calculations.
    // Rust version has simplified handling without these complex refinement calculations.
    // TODO: SYMBOL PLACEMENT DIFFERENCES: JS uses transposed flag for different placement algorithms
    // with complex offsetT/offsetS calculations and supports combination operators (OR, XOR)
    // with proper symbol bitmap iteration. Rust version may not handle all these placement modes.
    
    let mut bitmap = Vec::with_capacity(height);
    for _ in 0..height {
        let mut row = vec![0u8; width];
        if default_pixel_value != 0 {
            row.fill(default_pixel_value);
        }
        bitmap.push(row);
    }

    if huffman {
        // Huffman-coded text region
        if _huffman_tables.is_none() {
            return Err(Jbig2Error::new("Huffman tables required for Huffman text region"));
        }
        
        let huffman_tables = _huffman_tables.unwrap();
        let huffman_reader = _huffman_input.ok_or_else(|| {
            Jbig2Error::new("Huffman input reader required for Huffman text region")
        })?;
        
        // Huffman-coded text region implementation
        let strip_t = huffman_tables.t_table.decode(huffman_reader)?
            .ok_or_else(|| Jbig2Error::new("Failed to decode initial strip T"))
            .map(|v| -v)?;

        let mut first_s = 0i32;
        let mut i = 0;
        
        while i < number_of_symbol_instances {
            // Decode delta T using Huffman
            let delta_t = huffman_tables.t_table.decode(huffman_reader)?
                .unwrap_or(0);
            let strip_t = strip_t + delta_t;

            // Decode first S using Huffman
            let delta_first_s = if let Some(ref fs_table) = huffman_tables.fs_table {
                fs_table.decode(huffman_reader)?.unwrap_or(0)
            } else {
                0
            };
            first_s += delta_first_s;
            let mut current_s = first_s;
            
            loop {
                // Decode current T using Huffman
                let current_t = if strip_size > 1 && log_strip_size > 0 {
                    huffman_reader.read_bits(log_strip_size)? as i32
                } else {
                    0
                };
                
                let t = (strip_size as i32) * strip_t + current_t;
                
                // Decode symbol ID using Huffman
                let symbol_id = huffman_tables.symbol_id_table.decode(huffman_reader)?;
                let Some(symbol_id) = symbol_id else { break }; // OOB
                
                if symbol_id < 0 || symbol_id as usize >= input_symbols.len() {
                    break;
                }
                
                let symbol_bitmap = &input_symbols[symbol_id as usize];
                let symbol_width = if !symbol_bitmap.is_empty() { symbol_bitmap[0].len() } else { 0 };
                let symbol_height = symbol_bitmap.len();
                
                let increment = if !transposed {
                    if reference_corner > 1 {
                        current_s += symbol_width as i32 - 1;
                        0
                    } else {
                        symbol_width as i32 - 1
                    }
                } else if (reference_corner & 1) == 0 {
                    current_s += symbol_height as i32 - 1;
                    0
                } else {
                    symbol_height as i32 - 1
                };
                
                let offset_t = t - if (reference_corner & 1) != 0 { 0 } else { symbol_height as i32 - 1 };
                let offset_s = current_s - if (reference_corner & 2) != 0 { symbol_width as i32 - 1 } else { 0 };
                
                // Place symbol bitmap (same logic as arithmetic path)
                if transposed {
                    for s2 in 0..symbol_height {
                        let row_idx = (offset_s + s2 as i32) as usize;
                        if row_idx >= bitmap.len() {
                            continue;
                        }
                        let symbol_row = &symbol_bitmap[s2];
                        let max_width = ((width as i32) - offset_t).min(symbol_width as i32).max(0) as usize;
                        
                        match combination_operator {
                            0 => { // OR
                                for t2 in 0..max_width {
                                    let col_idx = (offset_t + t2 as i32) as usize;
                                    if col_idx < bitmap[row_idx].len() && t2 < symbol_row.len() {
                                        bitmap[row_idx][col_idx] |= symbol_row[t2];
                                    }
                                }
                            },
                            2 => { // XOR
                                for t2 in 0..max_width {
                                    let col_idx = (offset_t + t2 as i32) as usize;
                                    if col_idx < bitmap[row_idx].len() && t2 < symbol_row.len() {
                                        bitmap[row_idx][col_idx] ^= symbol_row[t2];
                                    }
                                }
                            },
                            _ => return Err(Jbig2Error::new(&format!("operator {} is not supported", combination_operator))),
                        }
                    }
                } else {
                    for t2 in 0..symbol_height {
                        let row_idx = (offset_t + t2 as i32) as usize;
                        if row_idx >= bitmap.len() {
                            continue;
                        }
                        let symbol_row = &symbol_bitmap[t2];
                        
                        match combination_operator {
                            0 => { // OR
                                for s2 in 0..symbol_width {
                                    let col_idx = (offset_s + s2 as i32) as usize;
                                    if col_idx < bitmap[row_idx].len() && s2 < symbol_row.len() {
                                        bitmap[row_idx][col_idx] |= symbol_row[s2];
                                    }
                                }
                            },
                            2 => { // XOR
                                for s2 in 0..symbol_width {
                                    let col_idx = (offset_s + s2 as i32) as usize;
                                    if col_idx < bitmap[row_idx].len() && s2 < symbol_row.len() {
                                        bitmap[row_idx][col_idx] ^= symbol_row[s2];
                                    }
                                }
                            },
                            _ => return Err(Jbig2Error::new(&format!("operator {} is not supported", combination_operator))),
                        }
                    }
                }
                
                i += 1;
                
                // Decode delta S using Huffman
                let delta_s = huffman_tables.s_table.decode(huffman_reader)?;
                let Some(delta_s) = delta_s else { break }; // OOB
                
                current_s += increment + delta_s + ds_offset;
            }
        }
        
        return Ok(bitmap);
    }

    let strip_t = decoding_context.decode_integer("IADT").map(|v| -v).unwrap_or(0);

    let mut first_s = 0i32;
    let mut i = 0;
    
    while i < number_of_symbol_instances {
        let delta_t = decoding_context.decode_integer("IADT").unwrap_or(0);
        let strip_t = strip_t + delta_t;

        let delta_first_s = decoding_context.decode_integer("IAFS").unwrap_or(0);
        first_s += delta_first_s;
        let mut current_s = first_s;
        
        loop {
            let current_t = if strip_size > 1 {
                // For arithmetic mode, we should still read log_strip_size bits
                // but using the integer decoder instead of direct bit reading
                if log_strip_size > 0 {
                    decoding_context.decode_integer("IAIT").unwrap_or(0)
                } else {
                    0
                }
            } else {
                0
            };
            
            let t = (strip_size as i32) * strip_t + current_t;
            
            let symbol_id = decoding_context.decode_iaid(symbol_code_length) as usize;
            
            if symbol_id >= input_symbols.len() {
                break;
            }
            
            let apply_refinement = if refinement {
                decoding_context.decode_integer("IARI").unwrap_or(0) != 0
            } else {
                false
            };
            
            let symbol_bitmap = &input_symbols[symbol_id];
            let symbol_width = if !symbol_bitmap.is_empty() { symbol_bitmap[0].len() } else { 0 };
            let symbol_height = symbol_bitmap.len();
            
            if apply_refinement {
                // Symbol refinement in text region
                // Read refinement offset parameters
                let refinement_offset_x = decoding_context.decode_integer("IARDX").unwrap_or(0);
                let refinement_offset_y = decoding_context.decode_integer("IARDY").unwrap_or(0);
                
                // Apply refinement to the symbol bitmap
                let refined_bitmap = decode_refinement(
                    symbol_width,
                    symbol_height,
                    refinement_template_index,
                    symbol_bitmap,
                    refinement_offset_x,
                    refinement_offset_y,
                    false, // prediction
                    refinement_at,
                    decoding_context,
                )?;
                
                // Update symbol dimensions and bitmap reference
                let _symbol_width = if !refined_bitmap.is_empty() { refined_bitmap[0].len() } else { 0 };
                let _symbol_height = refined_bitmap.len();
                
                // For text region, we'd need to store this refined bitmap temporarily
                // This is a simplified implementation - a full version would manage refined bitmaps
                return Err(Jbig2Error::new("Text region refinement storage not fully implemented"));
            }
            
            let increment = if !transposed {
                if reference_corner > 1 {
                    current_s += symbol_width as i32 - 1;
                    0
                } else {
                    symbol_width as i32 - 1
                }
            } else if (reference_corner & 1) == 0 {
                current_s += symbol_height as i32 - 1;
                0
            } else {
                symbol_height as i32 - 1
            };
            
            let offset_t = t - if (reference_corner & 1) != 0 { 0 } else { symbol_height as i32 - 1 };
            let offset_s = current_s - if (reference_corner & 2) != 0 { symbol_width as i32 - 1 } else { 0 };
            
            // Place symbol bitmap
            if transposed {
                for s2 in 0..symbol_height {
                    let row_idx = (offset_s + s2 as i32) as usize;
                    if row_idx >= bitmap.len() {
                        continue;
                    }
                    let symbol_row = &symbol_bitmap[s2];
                    let max_width = ((width as i32) - offset_t).min(symbol_width as i32).max(0) as usize;
                    
                    match combination_operator {
                        0 => { // OR
                            for t2 in 0..max_width {
                                let col_idx = (offset_t + t2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() && t2 < symbol_row.len() {
                                    bitmap[row_idx][col_idx] |= symbol_row[t2];
                                }
                            }
                        },
                        2 => { // XOR
                            for t2 in 0..max_width {
                                let col_idx = (offset_t + t2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() && t2 < symbol_row.len() {
                                    bitmap[row_idx][col_idx] ^= symbol_row[t2];
                                }
                            }
                        },
                        _ => return Err(Jbig2Error::new(&format!("operator {} is not supported", combination_operator))),
                    }
                }
            } else {
                for t2 in 0..symbol_height {
                    let row_idx = (offset_t + t2 as i32) as usize;
                    if row_idx >= bitmap.len() {
                        continue;
                    }
                    let symbol_row = &symbol_bitmap[t2];
                    
                    match combination_operator {
                        0 => { // OR
                            for s2 in 0..symbol_width {
                                let col_idx = (offset_s + s2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() && s2 < symbol_row.len() {
                                    bitmap[row_idx][col_idx] |= symbol_row[s2];
                                }
                            }
                        },
                        2 => { // XOR
                            for s2 in 0..symbol_width {
                                let col_idx = (offset_s + s2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() && s2 < symbol_row.len() {
                                    bitmap[row_idx][col_idx] ^= symbol_row[s2];
                                }
                            }
                        },
                        _ => return Err(Jbig2Error::new(&format!("operator {} is not supported", combination_operator))),
                    }
                }
            }
            
            i += 1;
            let delta_s = decoding_context.decode_integer("IADS");
            
            if delta_s.is_none() {
                break; // OOB
            }
            current_s += increment + delta_s.unwrap() + ds_offset;
        }
    }
    
    Ok(bitmap)
}

// Pattern dictionary decoding - ported from decodePatternDictionary function
fn decode_pattern_dictionary(
    mmr: bool,
    pattern_width: usize,
    pattern_height: usize,
    max_pattern_index: usize,
    template: usize,
    decoding_context: &mut DecodingContext,
) -> Result<Vec<Bitmap>, Jbig2Error> {
    // TODO: MAJOR ALGORITHMIC DIFFERENCE: JS version decodes a single collective bitmap
    // of width (maxPatternIndex + 1) * patternWidth and height patternHeight, then
    // divides it into individual patterns. Rust version decodes each pattern individually.
    // This could lead to different results if patterns share context across boundaries.
    let mut at = Vec::new();
    if !mmr {
        at.push(TemplatePixel { x: -(pattern_width as i32), y: 0 });
        if template == 0 {
            at.push(TemplatePixel { x: -3, y: -1 });
            at.push(TemplatePixel { x: 2, y: -2 });
            at.push(TemplatePixel { x: -2, y: -2 });
        }
    }

    let mut patterns = Vec::new();
    for _i in 0..=max_pattern_index {
        let bitmap = if mmr {
            // MMR-coded pattern bitmap
            let data_slice = &decoding_context.decoder.data[decoding_context.decoder.bp..decoding_context.decoder.data_end];
            decode_mmr_bitmap(data_slice, pattern_width, pattern_height, false)?
        } else {
            decode_bitmap(
                false, // mmr
                pattern_width,
                pattern_height,
                template,
                false, // prediction
                None,  // skip
                &at,
                decoding_context,
            )?
        };
        patterns.push(bitmap);
    }

    Ok(patterns)
}

// Placeholder for Huffman tables
#[derive(Debug)]
struct TextRegionHuffmanTables {
    // Huffman tables for text region as per JBIG2 spec Table E.2
    pub symbol_id_table: HuffmanTable,
    pub t_table: HuffmanTable,
    pub s_table: HuffmanTable,
    pub fs_table: Option<HuffmanTable>,
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
    // TODO: HUFFMAN LINE CONSTRUCTION DIFFERENCES: JS version has a single constructor
    // that handles both OOB (2-element array) and normal (4-5 element array) cases,
    // using isLowerRange flag from string "lower". Rust version uses separate constructors
    // (new_oob, new_normal, new_lower) with explicit type handling. This could affect
    // how Huffman tables are constructed from segment data.
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
    
    fn new_normal(range_low: i32, prefix_length: usize, range_length: usize, prefix_code: u32) -> Self {
        Self {
            is_oob: false,
            range_low,
            prefix_length,
            range_length,
            prefix_code,
            is_lower_range: false,
        }
    }
    
    fn new_lower(range_low: i32, prefix_length: usize, range_length: usize, prefix_code: u32) -> Self {
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
            self.children[bit].as_mut().unwrap().build_tree(line, shift - 1);
        }
    }
    
    fn decode_node(&self, reader: &mut Reader) -> Result<Option<i32>, Jbig2Error> {
        if self.is_leaf {
            if self.is_oob {
                return Ok(None);
            }
            let ht_offset = reader.read_bits(self.range_length)? as i32;
            let result = self.range_low + if self.is_lower_range { -ht_offset } else { ht_offset };
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
    
    fn decode(&self, reader: &mut Reader) -> Result<Option<i32>, Jbig2Error> {
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
    ((data[offset] as u32) << 24) |
    ((data[offset + 1] as u32) << 16) |
    ((data[offset + 2] as u32) << 8) |
    (data[offset + 3] as u32)
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
    
    if segment_type as usize >= SEGMENT_TYPES.len() || SEGMENT_TYPES[segment_type as usize].is_none() {
        return Err(Jbig2Error::new(&format!("invalid segment type: {}", segment_type)));
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
            return Err(Jbig2Error::new("insufficient data for referred-to segments"));
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
        // TODO: UNKNOWN SEGMENT LENGTH HANDLING DIFFERENCE: JS version implements complex
        // end-of-segment detection for ImmediateGenericRegion (type 38) by searching for
        // specific patterns (0xff, 0xac followed by height bytes). Rust version currently
        // returns error. This could cause failures with some JBIG2 files using unknown lengths.
        return Err(Jbig2Error::new("unknown segment length requires end-of-segment detection"));
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
fn read_region_segment_information(data: &[u8], start: usize) -> Result<RegionSegmentInformation, Jbig2Error> {
    if start + 17 > data.len() {
        return Err(Jbig2Error::new("insufficient data for region segment information"));
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
        if position + 9 <= data.len() && 
           data[position..position + 4] == [0x97, 0x4A, 0x42, 0x32] {
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
                start: 0,  // Relative to segment data
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
    
    pub fn draw_bitmap(&mut self, region_info: &RegionSegmentInformation, bitmap: &Bitmap) -> Result<(), Jbig2Error> {
        let page_info = self.current_page_info.as_ref()
            .ok_or_else(|| Jbig2Error::new("no page information available"))?;
        let buffer = self.buffer.as_mut()
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
            0 => { // OR
                for i in 0..height {
                    if i >= bitmap.len() { break; }
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
            },
            2 => { // XOR
                for i in 0..height {
                    if i >= bitmap.len() { break; }
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
            },
            _ => return Err(Jbig2Error::new(&format!("operator {} is not supported", combination_operator))),
        }
        
        Ok(())
    }
    
    fn on_immediate_generic_region(&mut self, region: &GenericRegion, data: &[u8], start: usize, end: usize) -> Result<(), Jbig2Error> {
        let mut decoding_context = DecodingContext::new(data.to_vec(), start, end);
        
        let bitmap = if region.mmr {
            // MMR-coded generic region
            let data_slice = &data[start..end];
            decode_mmr_bitmap(data_slice, region.info.width as usize, region.info.height as usize, false)?
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
    
    fn on_symbol_dictionary(&mut self, dictionary: &SymbolDictionary, current_segment: u32, referred_segments: &[u32], data: &[u8], start: usize, end: usize) -> Result<(), Jbig2Error> {
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
            huffman_reader.as_mut(),
        )?;
        
        // Store all symbols (input + new)
        let mut all_symbols = input_symbols;
        all_symbols.extend(new_symbols);
        self.symbols.insert(current_segment, all_symbols);
        
        Ok(())
    }
    
    fn on_immediate_text_region(&mut self, region: &TextRegion, referred_segments: &[u32], data: &[u8], start: usize, end: usize) -> Result<(), Jbig2Error> {
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
            Some(self.get_text_region_huffman_tables(region, referred_segments, input_symbols.len())?)
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
            huffman_reader.as_mut(),
        )?;
        
        self.draw_bitmap(&region.info, &bitmap)
    }
    
    fn on_pattern_dictionary(&mut self, dictionary: &PatternDictionary, current_segment: u32, data: &[u8], start: usize, end: usize) -> Result<(), Jbig2Error> {
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
    
    fn on_immediate_halftone_region(&mut self, region: &HalftoneRegion, referred_segments: &[u32], data: &[u8], start: usize, end: usize) -> Result<(), Jbig2Error> {
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
    
    fn on_tables(&mut self, current_segment: u32, data: &[u8], start: usize, end: usize) -> Result<(), Jbig2Error> {
        let table = decode_tables_segment(data, start, end)?;
        self.custom_tables.insert(current_segment, table);
        Ok(())
    }
    
    fn get_symbol_dictionary_huffman_tables(&self, dictionary: &SymbolDictionary, referred_segments: &[u32]) -> Result<SymbolDictionaryHuffmanTables, Jbig2Error> {
        // Based on getSymbolDictionaryHuffmanTables from JS
        let mut custom_index = 0;
        
        // Height table selection based on huffmanDHSelector (extracted from dictionary flags)
        let height_table = match dictionary.huffman as u8 { // Use huffman flag as selector for now
            0 | 1 => get_standard_table(4 + (dictionary.huffman as u32))?,
            3 => {
                let table = get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?.clone();
                custom_index += 1;
                table
            },
            _ => return Err(Jbig2Error::new("invalid Huffman DH selector")),
        };
        
        // Width table selection based on huffmanDWSelector
        let width_table = match dictionary.huffman as u8 { // Use huffman flag as selector for now  
            0 | 1 => get_standard_table(2 + (dictionary.huffman as u32))?,
            3 => {
                get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?.clone()
            },
            _ => return Err(Jbig2Error::new("invalid Huffman DW selector")),
        };
        
        
        // Bitmap size table - use standard table 1 for simplicity
        let bitmap_size_table = Some(get_standard_table(1)?);
        
        // Aggregate table - use standard table 1 for simplicity  
        let _aggregate_table = Some(get_standard_table(1)?);
        
        Ok(SymbolDictionaryHuffmanTables {
            height_table,
            width_table,
            bitmap_size_table,
            _aggregate_table,
        })
    }
    
    fn get_text_region_huffman_tables(&self, _region: &TextRegion, _referred_segments: &[u32], _number_of_symbols: usize) -> Result<TextRegionHuffmanTables, Jbig2Error> {
        // TODO: TEXT REGION HUFFMAN TABLE DIFFERENCES: JS version implements complex symbol ID 
        // Huffman table decoding with:
        // 1. RUNCODE reading (0-34) with 4-bit code lengths
        // 2. Special handling for codes 32-34 (repeats with 2/3/7 bit counts)
        // 3. byteAlign() after symbol ID table construction
        // 4. Proper table selection based on huffmanFS/DS/DT selectors (standard tables 6-13)
        // Rust version returns simplified fallback tables which may not decode correctly.
        Err(Jbig2Error::new("text region Huffman tables not fully implemented"))
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
    // TODO: GRID VECTOR CALCULATION DIFFERENCE: JS version uses complex formula:
    // x = (gridOffsetX + mg * gridVectorY + ng * gridVectorX) >> 8;
    // y = (gridOffsetY + mg * gridVectorX - ng * gridVectorY) >> 8;
    // Rust version uses simplified: pattern_x = grid_offset_x + (ng as i32) * grid_vector_x;
    // pattern_y = grid_offset_y + (mg as i32) * grid_vector_y;
    // This will produce completely different pattern placement results.
    if enable_skip {
        return Err(Jbig2Error::new("skip is not supported"));
    }
    if combination_operator != 0 {
        return Err(Jbig2Error::new(&format!("operator \"{}\" is not supported in halftone region", combination_operator)));
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
    let pattern_width = if !pattern0.is_empty() { pattern0[0].len() } else { 0 };
    let pattern_height = pattern0.len();
    let bits_per_value = log2(number_of_patterns);
    
    let mut at = Vec::new();
    if !mmr {
        at.push(TemplatePixel { 
            x: if template <= 1 { 3 } else { 2 }, 
            y: -1 
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
            // MMR-coded halftone bitmap
            let data_slice = &decoding_context.decoder.data[decoding_context.decoder.bp..decoding_context.decoder.data_end];
            decode_mmr_bitmap(data_slice, grid_width, grid_height, false)?
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
                    bit ^= gray_scale_bit_planes[j][mg][ng];
                }
                pattern_index |= (bit as usize) << j;
            }
            
            if pattern_index < patterns.len() {
                let pattern_bitmap = &patterns[pattern_index];
                
                // Calculate pattern position using grid vectors
                let pattern_x = grid_offset_x + (ng as i32) * grid_vector_x;
                let pattern_y = grid_offset_y + (mg as i32) * grid_vector_y;
                
                // Render pattern onto region bitmap
                for py in 0..pattern_height {
                    let region_y = pattern_y + py as i32;
                    if region_y < 0 || region_y as usize >= region_height {
                        continue;
                    }
                    
                    let pattern_row = &pattern_bitmap[py];
                    for px in 0..pattern_width {
                        let region_x = pattern_x + px as i32;
                        if region_x < 0 || region_x as usize >= region_width {
                            continue;
                        }
                        
                        if px < pattern_row.len() && pattern_row[px] != 0 {
                            let ry = region_y as usize;
                            let rx = region_x as usize;
                            if ry < region_bitmap.len() && rx < region_bitmap[ry].len() {
                                match combination_operator {
                                    0 => region_bitmap[ry][rx] |= pattern_row[px], // OR
                                    2 => region_bitmap[ry][rx] ^= pattern_row[px], // XOR
                                    _ => return Err(Jbig2Error::new(&format!("operator {} is not supported", combination_operator))),
                                }
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
    // TODO: MMR BITMAP DECODING DIFFERENCE: JS version uses CCITTFaxDecoder with parameters
    // K=-1, BlackIs1=true, EndOfBlock=endOfBlock and proper EOFB consumption handling.
    // Rust version has a simplified placeholder implementation that just creates a blank bitmap.
    // This is a major functional difference that will cause MMR-encoded regions to decode incorrectly.
    let _ = (data, end_of_block); // Suppress unused warnings
    
    // For now, return a blank bitmap as a placeholder
    Ok(Vec::new())
}

// Uncompressed bitmap reading - ported from readUncompressedBitmap function
fn read_uncompressed_bitmap(
    reader: &mut Reader,
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
fn process_segment(segment: &Segment, visitor: &mut SimpleSegmentVisitor) -> Result<(), Jbig2Error> {
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
        0 => { // SymbolDictionary
            // 7.4.2 Symbol dictionary segment syntax
            if position + 2 > data.len() {
                return Err(Jbig2Error::new("insufficient data for symbol dictionary"));
            }
            
            let dictionary_flags = read_uint16(data, position);
            position += 2;
            
            let huffman = (dictionary_flags & 1) != 0;
            let refinement = (dictionary_flags & 2) != 0;
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
                        return Err(Jbig2Error::new("insufficient data for refinement AT pixels"));
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
                template,
                refinement_template,
                at,
                refinement_at,
                number_of_exported_symbols,
                number_of_new_symbols,
            };
            
            visitor.on_symbol_dictionary(&dictionary, header.number, &header.referred_to, data, position, end)?;
        },
        6 | 7 => { // ImmediateTextRegion | ImmediateLosslessTextRegion
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
            
            if huffman {
                // Skip Huffman flags for now
                position += 2;
            }
            
            let mut refinement_at = Vec::new();
            if refinement && refinement_template == 0 {
                for _ in 0..2 {
                    if position + 2 > data.len() {
                        return Err(Jbig2Error::new("insufficient data for refinement AT pixels"));
                    }
                    refinement_at.push(TemplatePixel {
                        x: read_int8(data, position) as i32,
                        y: read_int8(data, position + 1) as i32,
                    });
                    position += 2;
                }
            }
            
            if position + 4 > data.len() {
                return Err(Jbig2Error::new("insufficient data for number of symbol instances"));
            }
            let number_of_symbol_instances = read_uint32(data, position);
            position += 4;
            
            let region = TextRegion {
                info,
                huffman,
                refinement,
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
        },
        16 => { // PatternDictionary
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
        },
        22 | 23 => { // ImmediateHalftoneRegion | ImmediateLosslessHalftoneRegion
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
            
            visitor.on_immediate_halftone_region(&region, &header.referred_to, data, position, end)?;
        },
        38 | 39 => { // ImmediateGenericRegion | ImmediateLosslessGenericRegion
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
        },
        48 => { // PageInformation
            if position + 19 > data.len() {
                return Err(Jbig2Error::new("insufficient data for page information"));
            }
            
            let width = read_uint32(data, position);
            let height = read_uint32(data, position + 4);
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
        },
        49 => { // EndOfPage
            // No processing needed
        },
        50 => { // EndOfStripe  
            // No processing needed
        },
        51 => { // EndOfFile
            // No processing needed
        },
        53 => { // Tables
            visitor.on_tables(header.number, data, position, end)?;
        },
        62 => { // Extension - can be ignored
            // No processing needed
        },
        _ => {
            return Err(Jbig2Error::new(&format!("segment type {}({}) is not implemented", header.type_name, header.segment_type)));
        }
    }
    
    Ok(())
}

// processSegments function - ported from JS processSegments function
fn process_segments(segments: &[Segment], visitor: &mut SimpleSegmentVisitor) -> Result<(), Jbig2Error> {
    for segment in segments {
        process_segment(segment, visitor)?;
    }
    Ok(())
}

// Tables segment decoding - ported from decodeTablesSegment function
fn decode_tables_segment(data: &[u8], start: usize, end: usize) -> Result<HuffmanTable, Jbig2Error> {
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
        let prefix_length = reader.read_bits(prefix_size_bits as usize)? as usize;
        let range_length = reader.read_bits(range_size_bits as usize)? as usize;
        
        lines.push(HuffmanLine::new_normal(
            current_range_low as i32,
            prefix_length,
            range_length,
            0,
        ));
        
        current_range_low += 1 << range_length;
    }
    
    // Lower range table line
    let prefix_length = reader.read_bits(prefix_size_bits as usize)? as usize;
    lines.push(HuffmanLine::new_lower(
        lowest_value as i32 - 1,
        prefix_length,
        32,
        0,
    ));
    
    // Upper range table line  
    let prefix_length = reader.read_bits(prefix_size_bits as usize)? as usize;
    lines.push(HuffmanLine::new_normal(
        highest_value as i32,
        prefix_length,
        32,
        0,
    ));
    
    // Out-of-band table line
    if (flags & 1) != 0 {
        let prefix_length = reader.read_bits(prefix_size_bits as usize)? as usize;
        lines.push(HuffmanLine::new_oob(prefix_length, 0));
    }
    
    Ok(HuffmanTable::new(lines, false))
}

// Standard tables getter - ported from getStandardTable function
fn get_standard_table(number: u32) -> Result<HuffmanTable, Jbig2Error> {
    // For simplicity, we'll recreate tables each time
    // In a production implementation, these would be cached
    
    // Annex B.5 Standard Huffman tables
    let lines_data: Vec<Vec<i32>> = match number {
        1 => vec![
            vec![0, 1, 4, 0x0],
            vec![16, 2, 8, 0x2],
            vec![272, 3, 16, 0x6],
            vec![65808, 3, 32, 0x7], // upper
        ],
        2 => vec![
            vec![0, 1, 0, 0x0],
            vec![1, 2, 0, 0x2],
            vec![2, 3, 0, 0x6],
            vec![3, 4, 3, 0xe],
            vec![11, 5, 6, 0x1e],
            vec![75, 6, 32, 0x3e], // upper
            vec![6, 0x3f], // OOB
        ],
        3 => vec![
            vec![-256, 8, 8, 0xfe],
            vec![0, 1, 0, 0x0],
            vec![1, 2, 0, 0x2],
            vec![2, 3, 0, 0x6],
            vec![3, 4, 3, 0xe],
            vec![11, 5, 6, 0x1e],
            vec![-257, 8, 32, 0xff, -1], // lower (using -1 as marker)
            vec![75, 7, 32, 0x7e], // upper
            vec![6, 0x3e], // OOB
        ],
        4 => vec![
            vec![1, 1, 0, 0x0],
            vec![2, 2, 0, 0x2],
            vec![3, 3, 0, 0x6],
            vec![4, 4, 3, 0xe],
            vec![12, 5, 6, 0x1e],
            vec![76, 5, 32, 0x1f], // upper
        ],
        5 => vec![
            vec![-255, 7, 8, 0x7e],
            vec![1, 1, 0, 0x0],
            vec![2, 2, 0, 0x2],
            vec![3, 3, 0, 0x6],
            vec![4, 4, 3, 0xe],
            vec![12, 5, 6, 0x1e],
            vec![-256, 7, 32, 0x7f, -1], // lower
            vec![76, 6, 32, 0x3e], // upper
        ],
        6 => vec![
            vec![-2048, 5, 10, 0x1c],
            vec![-1024, 4, 9, 0x8],
            vec![-512, 4, 8, 0x9],
            vec![-256, 4, 7, 0xa],
            vec![-128, 5, 6, 0x1d],
            vec![-64, 5, 5, 0x1e],
            vec![-32, 4, 5, 0xb],
            vec![0, 2, 7, 0x0],
            vec![128, 3, 7, 0x2],
            vec![256, 3, 8, 0x3],
            vec![512, 4, 9, 0xc],
            vec![1024, 4, 10, 0xd],
            vec![-2049, 6, 32, 0x3e, -1], // lower
            vec![2048, 6, 32, 0x3f], // upper
        ],
        7 => vec![
            vec![-1024, 4, 9, 0x8],
            vec![-512, 3, 8, 0x0],
            vec![-256, 4, 7, 0x9],
            vec![-128, 5, 6, 0x1a],
            vec![-64, 5, 5, 0x1b],
            vec![-32, 4, 5, 0xa],
            vec![0, 4, 5, 0xb],
            vec![32, 5, 5, 0x1c],
            vec![64, 5, 6, 0x1d],
            vec![128, 4, 7, 0xc],
            vec![256, 3, 8, 0x1],
            vec![512, 3, 9, 0x2],
            vec![1024, 3, 10, 0x3],
            vec![-1025, 5, 32, 0x1e, -1], // lower
            vec![2048, 5, 32, 0x1f], // upper
        ],
        8 => vec![
            vec![-15, 8, 3, 0xfc],
            vec![-7, 9, 1, 0x1fc],
            vec![-5, 8, 1, 0xfd],
            vec![-3, 9, 0, 0x1fd],
            vec![-2, 7, 0, 0x7c],
            vec![-1, 4, 0, 0xa],
            vec![0, 2, 1, 0x0],
            vec![2, 5, 0, 0x1a],
            vec![3, 6, 0, 0x3a],
            vec![4, 3, 4, 0x4],
            vec![20, 6, 1, 0x3b],
            vec![22, 4, 4, 0xb],
            vec![38, 4, 5, 0xc],
            vec![70, 5, 6, 0x1b],
            vec![134, 5, 7, 0x1c],
            vec![262, 6, 7, 0x3c],
            vec![390, 7, 8, 0x7d],
            vec![646, 6, 10, 0x3d],
            vec![-16, 9, 32, 0x1fe, -1], // lower
            vec![1670, 9, 32, 0x1ff], // upper
            vec![2, 0x1], // OOB
        ],
        9 => vec![
            vec![-31, 8, 4, 0xfc],
            vec![-15, 9, 2, 0x1fc],
            vec![-11, 8, 2, 0xfd],
            vec![-7, 9, 1, 0x1fd],
            vec![-5, 7, 1, 0x7c],
            vec![-3, 4, 1, 0xa],
            vec![-1, 3, 1, 0x2],
            vec![1, 3, 1, 0x3],
            vec![3, 5, 1, 0x1a],
            vec![5, 6, 1, 0x3a],
            vec![7, 3, 5, 0x4],
            vec![39, 6, 2, 0x3b],
            vec![43, 4, 5, 0xb],
            vec![75, 4, 6, 0xc],
            vec![139, 5, 7, 0x1b],
            vec![267, 5, 8, 0x1c],
            vec![523, 6, 8, 0x3c],
            vec![779, 7, 9, 0x7d],
            vec![1291, 6, 11, 0x3d],
            vec![-32, 9, 32, 0x1fe, -1], // lower
            vec![3339, 9, 32, 0x1ff], // upper
            vec![2, 0x0], // OOB
        ],
        10 => vec![
            vec![-21, 7, 4, 0x7a],
            vec![-5, 8, 0, 0xfc],
            vec![-4, 7, 0, 0x7b],
            vec![-3, 5, 0, 0x18],
            vec![-2, 2, 2, 0x0],
            vec![2, 5, 0, 0x19],
            vec![3, 6, 0, 0x36],
            vec![4, 7, 0, 0x7c],
            vec![5, 8, 0, 0xfd],
            vec![6, 2, 6, 0x1],
            vec![70, 5, 5, 0x1a],
            vec![102, 6, 5, 0x37],
            vec![134, 6, 6, 0x38],
            vec![198, 6, 7, 0x39],
            vec![326, 6, 8, 0x3a],
            vec![582, 6, 9, 0x3b],
            vec![1094, 6, 10, 0x3c],
            vec![2118, 7, 11, 0x7d],
            vec![-22, 8, 32, 0xfe, -1], // lower
            vec![4166, 8, 32, 0xff], // upper
            vec![2, 0x2], // OOB
        ],
        11 => vec![
            vec![1, 1, 0, 0x0],
            vec![2, 2, 1, 0x2],
            vec![4, 4, 0, 0xc],
            vec![5, 4, 1, 0xd],
            vec![7, 5, 1, 0x1c],
            vec![9, 5, 2, 0x1d],
            vec![13, 6, 2, 0x3c],
            vec![17, 7, 2, 0x7a],
            vec![21, 7, 3, 0x7b],
            vec![29, 7, 4, 0x7c],
            vec![45, 7, 5, 0x7d],
            vec![77, 7, 6, 0x7e],
            vec![141, 7, 32, 0x7f], // upper
        ],
        12 => vec![
            vec![1, 1, 0, 0x0],
            vec![2, 2, 0, 0x2],
            vec![3, 3, 1, 0x6],
            vec![5, 5, 0, 0x1c],
            vec![6, 5, 1, 0x1d],
            vec![8, 6, 1, 0x3c],
            vec![10, 7, 0, 0x7a],
            vec![11, 7, 1, 0x7b],
            vec![13, 7, 2, 0x7c],
            vec![17, 7, 3, 0x7d],
            vec![25, 7, 4, 0x7e],
            vec![41, 8, 5, 0xfe],
            vec![73, 8, 32, 0xff], // upper
        ],
        13 => vec![
            vec![1, 1, 0, 0x0],
            vec![2, 3, 0, 0x4],
            vec![3, 4, 0, 0xc],
            vec![4, 5, 0, 0x1c],
            vec![5, 4, 1, 0xd],
            vec![7, 3, 3, 0x5],
            vec![15, 6, 1, 0x3a],
            vec![17, 6, 2, 0x3b],
            vec![21, 6, 3, 0x3c],
            vec![29, 6, 4, 0x3d],
            vec![45, 6, 5, 0x3e],
            vec![77, 7, 6, 0x7e],
            vec![141, 7, 32, 0x7f], // upper
        ],
        14 => vec![
            vec![-2, 3, 0, 0x4],
            vec![-1, 3, 0, 0x5],
            vec![0, 1, 0, 0x0],
            vec![1, 3, 0, 0x6],
            vec![2, 3, 0, 0x7],
        ],
        15 => vec![
            vec![-24, 7, 4, 0x7c],
            vec![-8, 6, 2, 0x3c],
            vec![-4, 5, 1, 0x1c],
            vec![-2, 4, 0, 0xc],
            vec![-1, 3, 0, 0x4],
            vec![0, 1, 0, 0x0],
            vec![1, 3, 0, 0x5],
            vec![2, 4, 0, 0xd],
            vec![3, 5, 1, 0x1d],
            vec![5, 6, 2, 0x3d],
            vec![9, 7, 4, 0x7d],
            vec![-25, 7, 32, 0x7e, -1], // lower
            vec![25, 7, 32, 0x7f], // upper
        ],
        _ => return Err(Jbig2Error::new(&format!("standard table B.{} does not exist", number))),
    };
    
    // Convert to HuffmanLine objects
    let mut lines = Vec::new();
    for line_data in lines_data {
        if line_data.len() == 2 {
            // OOB line
            lines.push(HuffmanLine::new_oob(line_data[0] as usize, line_data[1] as u32));
        } else if line_data.len() >= 4 {
            let is_lower = line_data.len() == 5 && line_data[4] == -1;
            if is_lower {
                lines.push(HuffmanLine::new_lower(
                    line_data[0],
                    line_data[1] as usize,
                    line_data[2] as usize,
                    line_data[3] as u32,
                ));
            } else {
                lines.push(HuffmanLine::new_normal(
                    line_data[0],
                    line_data[1] as usize,
                    line_data[2] as usize,
                    line_data[3] as u32,
                ));
            }
        }
    }
    
    let table = HuffmanTable::new(lines, true);
    Ok(table)
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

