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

#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]

use crate::filter::ccitt::{CCITTFaxDecoder, CCITTFaxDecoderOptions};
use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;
use hayro_common::byte::Reader as CrateReader;
use log::warn;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

pub(crate) fn decode(data: &[u8], params: Dict<'_>) -> Option<Vec<u8>> {
    let globals = params.get::<Stream<'_>>(JBIG2_GLOBALS);

    let mut chunks = Vec::new();

    // std::fs::write("out.jb2", data);

    if let Some(globals_data) = globals.and_then(|g| g.decoded().ok()) {
        // std::fs::write("globals_data.jb2", &globals_data);
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

    // JBIG2 had black as 1 and white as 0, inverting the colors.
    for b in &mut buf {
        *b ^= 0xFF;
    }

    Some(buf)
}

#[derive(Debug)]
struct Jbig2Error {
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
    data: Vec<u8>,
    start: usize,
    end: usize,
    decoder: ArithmeticDecoder,
    context_cache: ContextCache,
}

impl DecodingContext {
    fn new(data: Vec<u8>, start: usize, end: usize) -> Self {
        Self {
            data: data.clone(),
            start,
            end,
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
}

#[derive(Clone)]
struct Chunk {
    data: Vec<u8>,
    start: usize,
    end: usize,
}

#[derive(Debug, Default)]
struct Jbig2Header {
    random_access: bool,
}

fn read_segments(
    header: &Jbig2Header,
    data: &[u8],
    start: usize,
    end: usize,
) -> Result<Vec<Segment>, Jbig2Error> {
    let owned_data = Rc::new(data[start..end].to_vec());
    let mut segments = Vec::new();
    let mut position = start;

    while position < end {
        let segment_header = read_segment_header(data, position)?;

        position = segment_header.header_end;

        let mut segment = Segment {
            header: segment_header.clone(),
            data: owned_data.clone(), // Share reference to original data like JS
            start: 0,                 // Will be set below
            end: 0,                   // Will be set below
        };

        if !header.random_access {
            // Set segment positions immediately during parsing (non-random access)
            segment.start = position;
            position += segment_header.length as usize;
            segment.end = position;
        }

        segments.push(segment);

        // Break on end of file segment
        if segment_header.segment_type == 51 {
            break;
        }
    }

    if header.random_access {
        // Defer position setting until all segments are read (random access)
        for segment in &mut segments {
            segment.start = position;
            position += segment.header.length as usize;
            segment.end = position;
        }
    }

    Ok(segments)
}

#[derive(Clone, Debug)]
struct ArithmeticDecoder {
    data: Vec<u8>,
    bp: usize,
    data_end: usize,
    chigh: u32,
    clow: u32,
    ct: i32,
    a: u32,
    // counter: usize,
}

impl ArithmeticDecoder {
    // C.3.5 Initialisation of the decoder (INITDEC)
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
            // counter: 0,
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

    if let Some(signed_value) = signed_value
        && (MIN_INT_32..=MAX_INT_32).contains(&signed_value)
    {
        return Some(signed_value);
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

// Constants for segment types (7.3 Segment types) - matches JS SegmentTypes array exactly
const SEGMENT_TYPES: &[Option<&str>] = &[
    Some("SymbolDictionary"),
    None,
    None,
    None,
    Some("IntermediateTextRegion"),
    None,
    Some("ImmediateTextRegion"),
    Some("ImmediateLosslessTextRegion"),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some("PatternDictionary"),
    None,
    None,
    None,
    Some("IntermediateHalftoneRegion"),
    None,
    Some("ImmediateHalftoneRegion"),
    Some("ImmediateLosslessHalftoneRegion"),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some("IntermediateGenericRegion"),
    None,
    Some("ImmediateGenericRegion"),
    Some("ImmediateLosslessGenericRegion"),
    Some("IntermediateGenericRefinementRegion"),
    None,
    Some("ImmediateGenericRefinementRegion"),
    Some("ImmediateLosslessGenericRefinementRegion"),
    None,
    None,
    None,
    None,
    Some("PageInformation"),
    Some("EndOfPage"),
    Some("EndOfStripe"),
    Some("EndOfFile"),
    Some("Profiles"),
    Some("Tables"),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some("Extension"),
];

// Coding templates
const CODING_TEMPLATES: [&[[i32; 2]]; 4] = [
    &[
        [-1, -2],
        [0, -2],
        [1, -2],
        [-2, -1],
        [-1, -1],
        [0, -1],
        [1, -1],
        [2, -1],
        [-4, 0],
        [-3, 0],
        [-2, 0],
        [-1, 0],
    ],
    &[
        [-1, -2],
        [0, -2],
        [1, -2],
        [2, -2],
        [-2, -1],
        [-1, -1],
        [0, -1],
        [1, -1],
        [2, -1],
        [-3, 0],
        [-2, 0],
        [-1, 0],
    ],
    &[
        [-1, -2],
        [0, -2],
        [1, -2],
        [-2, -1],
        [-1, -1],
        [0, -1],
        [1, -1],
        [-2, 0],
        [-1, 0],
    ],
    &[
        [-3, -1],
        [-2, -1],
        [-1, -1],
        [0, -1],
        [1, -1],
        [-4, 0],
        [-3, 0],
        [-2, 0],
        [-1, 0],
    ],
];

// Refinement templates
const REFINEMENT_TEMPLATES: [RefinementTemplate; 2] = [
    RefinementTemplate {
        coding: &[[0, -1], [1, -1], [-1, 0]],
        reference: &[
            [0, -1],
            [1, -1],
            [-1, 0],
            [0, 0],
            [1, 0],
            [-1, 1],
            [0, 1],
            [1, 1],
        ],
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

// QM Coder Table C-2 from JPEG 2000 Part I Final Committee Draft Version 1.0
#[derive(Clone, Copy)]
struct QeEntry {
    qe: u32,
    nmps: u8,
    nlps: u8,
    switch_flag: u8,
}

const QE_TABLE: [QeEntry; 47] = [
    QeEntry {
        qe: 0x5601,
        nmps: 1,
        nlps: 1,
        switch_flag: 1,
    },
    QeEntry {
        qe: 0x3401,
        nmps: 2,
        nlps: 6,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1801,
        nmps: 3,
        nlps: 9,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0ac1,
        nmps: 4,
        nlps: 12,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0521,
        nmps: 5,
        nlps: 29,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0221,
        nmps: 38,
        nlps: 33,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x5601,
        nmps: 7,
        nlps: 6,
        switch_flag: 1,
    },
    QeEntry {
        qe: 0x5401,
        nmps: 8,
        nlps: 14,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x4801,
        nmps: 9,
        nlps: 14,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3801,
        nmps: 10,
        nlps: 14,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3001,
        nmps: 11,
        nlps: 17,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x2401,
        nmps: 12,
        nlps: 18,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1c01,
        nmps: 13,
        nlps: 20,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1601,
        nmps: 29,
        nlps: 21,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x5601,
        nmps: 15,
        nlps: 14,
        switch_flag: 1,
    },
    QeEntry {
        qe: 0x5401,
        nmps: 16,
        nlps: 14,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x5101,
        nmps: 17,
        nlps: 15,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x4801,
        nmps: 18,
        nlps: 16,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3801,
        nmps: 19,
        nlps: 17,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3401,
        nmps: 20,
        nlps: 18,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3001,
        nmps: 21,
        nlps: 19,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x2801,
        nmps: 22,
        nlps: 19,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x2401,
        nmps: 23,
        nlps: 20,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x2201,
        nmps: 24,
        nlps: 21,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1c01,
        nmps: 25,
        nlps: 22,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1801,
        nmps: 26,
        nlps: 23,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1601,
        nmps: 27,
        nlps: 24,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1401,
        nmps: 28,
        nlps: 25,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1201,
        nmps: 29,
        nlps: 26,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1101,
        nmps: 30,
        nlps: 27,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0ac1,
        nmps: 31,
        nlps: 28,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x09c1,
        nmps: 32,
        nlps: 29,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x08a1,
        nmps: 33,
        nlps: 30,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0521,
        nmps: 34,
        nlps: 31,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0441,
        nmps: 35,
        nlps: 32,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x02a1,
        nmps: 36,
        nlps: 33,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0221,
        nmps: 37,
        nlps: 34,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0141,
        nmps: 38,
        nlps: 35,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0111,
        nmps: 39,
        nlps: 36,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0085,
        nmps: 40,
        nlps: 37,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0049,
        nmps: 41,
        nlps: 38,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0025,
        nmps: 42,
        nlps: 39,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0015,
        nmps: 43,
        nlps: 40,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0009,
        nmps: 44,
        nlps: 41,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0005,
        nmps: 45,
        nlps: 42,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0001,
        nmps: 45,
        nlps: 43,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x5601,
        nmps: 46,
        nlps: 46,
        switch_flag: 0,
    },
];

fn decode_bitmap_template0(
    width: usize,
    height: usize,
    decoding_context: &mut DecodingContext,
) -> Bitmap {
    let contexts = decoding_context.context_cache.get_contexts("GB");
    let decoder = &mut decoding_context.decoder;
    let mut bitmap = Vec::with_capacity(height);

    // ...ooooo....
    // ..ooooooo... Context template for current pixel (X)
    // .ooooX...... (concatenate values of 'o'-pixels to get contextLabel)
    const OLD_PIXEL_MASK: u32 = 0x7bf7; // 01111 0111111 0111

    for i in 0..height {
        let row = Rc::new(RefCell::new(vec![0_u8; width]));
        bitmap.push(row.clone());
        let row1 = if i < 1 {
            row.clone()
        } else {
            bitmap[i - 1].clone()
        };
        let row2 = if i < 2 {
            row.clone()
        } else {
            bitmap[i - 2].clone()
        };

        // At the beginning of each row:
        // Fill contextLabel with pixels that are above/right of (X)
        let mut context_label = (row2.borrow()[0] as u32) << 13
            | (row2.borrow().get(1).copied().unwrap_or(0) as u32) << 12
            | (row2.borrow().get(2).copied().unwrap_or(0) as u32) << 11
            | (row1.borrow().first().copied().unwrap_or(0) as u32) << 7
            | (row1.borrow().get(1).copied().unwrap_or(0) as u32) << 6
            | (row1.borrow().get(2).copied().unwrap_or(0) as u32) << 5
            | (row1.borrow().get(3).copied().unwrap_or(0) as u32) << 4;

        for j in 0..width {
            let pixel = decoder.read_bit(contexts, context_label as usize);
            row.borrow_mut()[j] = pixel;

            // At each pixel: Clear contextLabel pixels that are shifted
            // out of the context, then add new ones.

            context_label = ((context_label & OLD_PIXEL_MASK) << 1)
                | {
                    if j + 3 < width {
                        (row2.borrow()[j + 3] as u32) << 11
                    } else {
                        0
                    }
                }
                | {
                    if j + 4 < width {
                        (row1.borrow()[j + 4] as u32) << 4
                    } else {
                        0
                    }
                }
                | pixel as u32;
        }
    }

    bitmap.iter().map(|i| i.borrow().clone()).collect()
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
    if mmr {
        let reader = Reader::new(
            &decoding_context.data,
            decoding_context.start,
            decoding_context.end,
        );
        return decode_mmr_bitmap(&reader, width, height, false);
    }

    // Use optimized version for the most common case
    if template_index == 0
        && skip.is_none()
        && !prediction
        && at.len() == 4
        && at[0].x == 3
        && at[0].y == -1
        && at[1].x == -3
        && at[1].y == -1
        && at[2].x == 2
        && at[2].y == -2
        && at[3].x == -2
        && at[3].y == -2
    {
        return Ok(decode_bitmap_template0(width, height, decoding_context));
    }

    let use_skip = skip.is_some();
    let mut template = CODING_TEMPLATES[template_index]
        .iter()
        .map(|[x, y]| TemplatePixel { x: *x, y: *y })
        .collect::<Vec<_>>();
    template.extend_from_slice(at);

    // Sorting is non-standard, and it is not required. But sorting increases
    // the number of template bits that can be reused from the previous
    // contextLabel in the main loop.
    template.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));

    let template_length = template.len();

    let mut template_x: Vec<i8> = vec![0; template_length];
    let mut template_y: Vec<i8> = vec![0; template_length];
    let mut changing_template_entries = Vec::new();
    let mut reuse_mask = 0_u32;
    let mut min_x = 0_i32;
    let mut max_x = 0_i32;
    let mut min_y = 0_i32;

    for k in 0..template_length {
        template_x[k] = template[k].x as i8;
        template_y[k] = template[k].y as i8;

        min_x = min_x.min(template[k].x);
        max_x = max_x.max(template[k].x);
        min_y = min_y.min(template[k].y);

        // Check if the template pixel appears in two consecutive context labels,
        // so it can be reused. Otherwise, we add it to the list of changing
        // template entries.
        if k < template_length - 1
            && template[k].y == template[k + 1].y
            && template[k].x == template[k + 1].x - 1
        {
            reuse_mask |= 1 << (template_length - 1 - k);
        } else {
            changing_template_entries.push(k);
        }
    }

    let changing_entries_length = changing_template_entries.len();

    let changing_template_x: Vec<i8> = changing_template_entries
        .iter()
        .map(|&k| template[k].x as i8)
        .collect();
    let changing_template_y: Vec<i8> = changing_template_entries
        .iter()
        .map(|&k| template[k].y as i8)
        .collect();
    let changing_template_bit: Vec<u16> = changing_template_entries
        .iter()
        .map(|&k| 1_u16 << (template_length - 1 - k))
        .collect();

    // Get the safe bounding box edges from the width, height, minX, maxX, minY
    let sbb_left = -min_x;
    let sbb_top = -min_y;
    let sbb_right = width as i32 - max_x;

    let pseudo_pixel_context = REUSED_CONTEXTS[template_index];
    let mut bitmap = Vec::with_capacity(height);
    let mut row = Rc::new(RefCell::new(vec![0_u8; width]));

    let decoder = &mut decoding_context.decoder;
    let contexts = decoding_context.context_cache.get_contexts("GB");

    let mut ltp = 0_u8;
    let mut context_label = 0_u32;

    for i in 0..height {
        if prediction {
            let sltp = decoder.read_bit(contexts, pseudo_pixel_context as usize);
            ltp ^= sltp;

            if ltp != 0 {
                bitmap.push(row.clone()); // duplicate previous row
                continue;
            }
        }

        let old_data = row.borrow().clone();
        row = Rc::new(RefCell::new(old_data));
        bitmap.push(row.clone());

        for j in 0..width {
            if use_skip && skip.unwrap()[i][j] != 0 {
                row.borrow_mut()[j] = 0;
                continue;
            }

            // Are we in the middle of a scanline, so we can reuse contextLabel bits?
            if (j as i32) >= sbb_left && (j as i32) < sbb_right && (i as i32) >= sbb_top {
                // If yes, we can just shift the bits that are reusable and only
                // fetch the remaining ones.
                context_label = (context_label << 1) & reuse_mask;
                for k in 0..changing_entries_length {
                    let i0 = (i as i32 + changing_template_y[k] as i32) as usize;
                    let j0 = (j as i32 + changing_template_x[k] as i32) as usize;
                    let bit = bitmap[i0].borrow()[j0];
                    if bit != 0 {
                        context_label |= changing_template_bit[k] as u32;
                    }
                }
            } else {
                // compute the contextLabel from scratch
                context_label = 0;
                let mut shift = template_length - 1;
                for k in 0..template_length {
                    let j0 = j as i32 + template_x[k] as i32;
                    if j0 >= 0 && j0 < width as i32 {
                        let i0 = i as i32 + template_y[k] as i32;
                        if i0 >= 0 {
                            let bit = bitmap[i0 as usize].borrow()[j0 as usize];
                            if bit != 0 {
                                context_label |= (bit as u32) << shift;
                            }
                        }
                    }

                    shift = shift.saturating_sub(1);
                }
            }

            let pixel = decoder.read_bit(contexts, context_label as usize);
            row.borrow_mut()[j] = pixel;
        }
    }

    Ok(bitmap.into_iter().map(|i| i.borrow().clone()).collect())
}

// 6.3.2 Generic Refinement Region Decoding Procedure
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

    let mut coding_template_x = vec![0_i32; coding_template_length];
    let mut coding_template_y = vec![0_i32; coding_template_length];
    for k in 0..coding_template_length {
        coding_template_x[k] = coding_template[k][0];
        coding_template_y[k] = coding_template[k][1];
    }

    let mut reference_template: Vec<[i32; 2]> =
        REFINEMENT_TEMPLATES[template_index].reference.to_vec();
    if template_index == 0 {
        reference_template.push([at[1].x, at[1].y]);
    }
    let reference_template_length = reference_template.len();

    let mut reference_template_x = vec![0_i32; reference_template_length];
    let mut reference_template_y = vec![0_i32; reference_template_length];
    for k in 0..reference_template_length {
        reference_template_x[k] = reference_template[k][0];
        reference_template_y[k] = reference_template[k][1];
    }

    let reference_width = reference_bitmap[0].len();
    let reference_height = reference_bitmap.len();

    let pseudo_pixel_context = REFINEMENT_REUSED_CONTEXTS[template_index];
    let mut bitmap = vec![];

    let decoder = &mut decoding_context.decoder;
    let contexts = decoding_context.context_cache.get_contexts("GR");

    let mut ltp = 0_u8;

    for i in 0..height {
        if prediction {
            let sltp = decoder.read_bit(contexts, pseudo_pixel_context as usize);
            ltp ^= sltp;
            if ltp != 0 {
                return Err(Jbig2Error::new("prediction is not supported"));
            }
        }

        let row = Rc::new(RefCell::new(vec![0_u8; width]));
        bitmap.push(row.clone());

        for j in 0..width {
            let mut context_label = 0_u32;

            for k in 0..coding_template_length {
                let i0 = i as i32 + coding_template_y[k];
                let j0 = j as i32 + coding_template_x[k];

                if i0 < 0 || j0 < 0 || j0 >= width as i32 {
                    context_label <<= 1; // out of bound pixel
                } else {
                    context_label =
                        (context_label << 1) | (bitmap[i0 as usize].borrow()[j0 as usize] as u32);
                }
            }

            for k in 0..reference_template_length {
                let i0 = i as i32 + reference_template_y[k] - offset_y;
                let j0 = j as i32 + reference_template_x[k] - offset_x;

                if i0 < 0 || i0 >= reference_height as i32 || j0 < 0 || j0 >= reference_width as i32
                {
                    context_label <<= 1; // out of bound pixel
                } else {
                    context_label =
                        (context_label << 1) | (reference_bitmap[i0 as usize][j0 as usize] as u32);
                }
            }

            let pixel = decoder.read_bit(contexts, context_label as usize);
            row.borrow_mut()[j] = pixel;
        }
    }

    Ok(bitmap.into_iter().map(|i| i.borrow().clone()).collect())
}

// 6.5.5 Decoding the symbol dictionary
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
    huffman_input: Option<&Reader<'_>>,
) -> Result<Vec<Bitmap>, Jbig2Error> {
    if huffman && refinement {
        return Err(Jbig2Error::new(
            "symbol refinement with Huffman is not supported",
        ));
    }

    let mut new_symbols = Vec::new();
    let mut current_height = 0_i32;
    let mut symbol_code_length = log2(symbols.len() + number_of_new_symbols);

    let table_b1 = if huffman {
        Some(get_standard_table(1)?) // standard table B.1
    } else {
        None
    };
    let mut symbol_widths = Vec::new();
    if huffman {
        symbol_code_length = symbol_code_length.max(1); // 6.5.8.2.3
    }

    while new_symbols.len() < number_of_new_symbols {
        // Delta height decoding
        let delta_height = if huffman {
            huffman_tables
                .as_ref()
                .ok_or_else(|| Jbig2Error::new("Huffman tables required"))?
                .height_table
                .decode(
                    huffman_input
                        .as_ref()
                        .ok_or_else(|| Jbig2Error::new("Huffman input required"))?,
                )
                .map_err(|_| Jbig2Error::new("Failed to decode delta height"))?
                .ok_or_else(|| Jbig2Error::new("Got OOB for delta height"))?
        } else {
            decoding_context
                .decode_integer("IADH") // 6.5.6
                .ok_or_else(|| Jbig2Error::new("Failed to decode IADH"))?
        };
        current_height += delta_height;

        let mut current_width = 0_i32;
        let mut total_width = 0_i32;
        let first_symbol = if huffman { symbol_widths.len() } else { 0 };

        loop {
            let delta_width = if huffman {
                huffman_tables
                    .as_ref()
                    .unwrap()
                    .width_table
                    .decode(huffman_input.unwrap())?
            } else {
                decoding_context.decode_integer("IADW") // 6.5.7
            };

            let Some(dw) = delta_width else { break }; // OOB
            current_width += dw;
            total_width += current_width;

            if refinement {
                // 6.5.8.2 Refinement/aggregate-coded symbol bitmap
                let number_of_instances = decoding_context
                    .decode_integer("IAAI")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode IAAI"))?;

                let bitmap = if number_of_instances > 1 {
                    // Multiple instances - call text region
                    let mut all_symbols = symbols.to_vec();
                    all_symbols.extend_from_slice(&new_symbols);

                    decode_text_region(
                        huffman,
                        refinement,
                        current_width as usize,
                        current_height as usize,
                        0,
                        number_of_instances as usize,
                        1,
                        &all_symbols,
                        symbol_code_length,
                        false,
                        0,
                        1,
                        0,
                        huffman_tables.map(|_| unreachable!("no text region huffman tables")),
                        refinement_template_index,
                        refinement_at,
                        decoding_context,
                        0,
                        huffman_input,
                    )?
                } else {
                    let symbol_id = decoding_context.decode_iaid(symbol_code_length) as usize;
                    let rdx = decoding_context
                        .decode_integer("IARDX") // 6.4.11.3
                        .ok_or_else(|| Jbig2Error::new("Failed to decode IARDX"))?;
                    let rdy = decoding_context
                        .decode_integer("IARDY") // 6.4.11.4
                        .ok_or_else(|| Jbig2Error::new("Failed to decode IARDY"))?;

                    let symbol = if symbol_id < symbols.len() {
                        &symbols[symbol_id]
                    } else {
                        &new_symbols[symbol_id - symbols.len()]
                    };

                    decode_refinement(
                        current_width as usize,
                        current_height as usize,
                        refinement_template_index,
                        symbol,
                        rdx,
                        rdy,
                        false,
                        refinement_at,
                        decoding_context,
                    )?
                };
                new_symbols.push(bitmap);
            } else if huffman {
                // Store only symbol width and decode a collective bitmap when the height class is done.
                symbol_widths.push(current_width);
            } else {
                // 6.5.8.1 Direct-coded symbol bitmap
                let bitmap = decode_bitmap(
                    false,
                    current_width as usize,
                    current_height as usize,
                    template_index,
                    false,
                    None,
                    at,
                    decoding_context,
                )?;
                new_symbols.push(bitmap);
            }
        }

        if huffman && !refinement {
            let huffman_input = huffman_input.unwrap();

            // 6.5.9 Height class collective bitmap
            let bitmap_size = huffman_tables
                .as_ref()
                .unwrap()
                .bitmap_size_table
                .as_ref()
                .ok_or_else(|| Jbig2Error::new("Bitmap size table required"))?
                .decode(huffman_input)?
                .ok_or_else(|| Jbig2Error::new("Got OOB for bitmap size"))?;

            huffman_input.byte_align();

            let collective_bitmap = if bitmap_size == 0 {
                // Uncompressed collective bitmap
                read_uncompressed_bitmap(
                    huffman_input,
                    total_width as usize,
                    current_height as usize,
                )?
            } else {
                // MMR collective bitmap
                let mut input = huffman_input.0.borrow_mut();
                let original_end = input.end;
                let bitmap_end = input.position + bitmap_size as usize;
                input.end = bitmap_end;
                drop(input);

                let result = decode_mmr_bitmap(
                    huffman_input,
                    total_width as usize,
                    current_height as usize,
                    false,
                );

                let mut input = huffman_input.0.borrow_mut();
                input.end = original_end;
                input.position = bitmap_end;

                result?
            };

            let number_of_symbols_decoded = symbol_widths.len();
            if first_symbol == number_of_symbols_decoded - 1 {
                // collectiveBitmap is a single symbol.
                new_symbols.push(collective_bitmap);
            } else {
                // Divide collectiveBitmap into symbols.
                let mut x_min = 0;
                for i in first_symbol..number_of_symbols_decoded {
                    let bitmap_width = symbol_widths[i] as usize;
                    let x_max = x_min + bitmap_width;
                    let mut symbol_bitmap = Vec::new();
                    for y in 0..(current_height as usize) {
                        symbol_bitmap.push(collective_bitmap[y][x_min..x_max].to_vec());
                    }
                    new_symbols.push(symbol_bitmap);
                    x_min = x_max;
                }
            }
        }
    }

    // 6.5.10 Exported symbols
    let mut exported_symbols = Vec::new();
    let mut flags = Vec::new();
    let mut current_flag = false;
    let total_symbols_length = symbols.len() + number_of_new_symbols;

    while flags.len() < total_symbols_length {
        let run_length = if huffman {
            let huffman_input = huffman_input.unwrap();

            table_b1
                .as_ref()
                .unwrap()
                .decode(huffman_input)?
                .ok_or_else(|| Jbig2Error::new("Got OOB for run length"))?
        } else {
            decoding_context
                .decode_integer("IAEX")
                .ok_or_else(|| Jbig2Error::new("Failed to decode IAEX"))?
        };

        for _ in 0..run_length {
            flags.push(current_flag);
        }
        current_flag = !current_flag;
    }

    // Export symbols based on flags
    for (i, &flag) in flags.iter().enumerate().take(symbols.len()) {
        if flag {
            exported_symbols.push(symbols[i].clone());
        }
    }

    for (j, symbol) in new_symbols.iter().enumerate() {
        let i = symbols.len() + j;
        if i < flags.len() && flags[i] {
            exported_symbols.push(symbol.clone());
        }
    }

    Ok(exported_symbols)
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
    huffman_tables: Option<&TextRegionHuffmanTables>,
    refinement_template_index: usize,
    refinement_at: &[TemplatePixel],
    decoding_context: &mut DecodingContext,
    log_strip_size: usize,
    huffman_input: Option<&Reader<'_>>,
) -> Result<Bitmap, Jbig2Error> {
    if huffman && refinement {
        return Err(Jbig2Error::new("refinement with Huffman is not supported"));
    }

    // Prepare bitmap
    let mut bitmap = Vec::new();
    for _ in 0..height {
        let mut row = vec![0_u8; width];
        if default_pixel_value != 0 {
            row.fill(default_pixel_value);
        }
        bitmap.push(row);
    }

    let mut strip_t = if huffman {
        -huffman_tables
            .unwrap()
            .table_delta_t
            .decode(huffman_input.unwrap())?
            .ok_or_else(|| Jbig2Error::new("Failed to decode initial stripT"))?
    } else {
        -decoding_context
            .decode_integer("IADT")
            .ok_or_else(|| Jbig2Error::new("Failed to decode initial stripT"))?
    };

    let mut first_s = 0_i32;
    let mut i = 0;

    while i < number_of_symbol_instances {
        let delta_t = if huffman {
            huffman_tables
                .unwrap()
                .table_delta_t
                .decode(huffman_input.unwrap())?
                .ok_or_else(|| Jbig2Error::new("Failed to decode deltaT"))?
        } else {
            decoding_context
                .decode_integer("IADT")
                .ok_or_else(|| Jbig2Error::new("Failed to decode deltaT"))?
        };
        strip_t += delta_t;

        let delta_first_s = if huffman {
            huffman_tables
                .unwrap()
                .table_first_s
                .as_ref()
                .unwrap()
                .decode(huffman_input.unwrap())?
                .ok_or_else(|| Jbig2Error::new("Failed to decode deltaFirstS"))?
        } else {
            decoding_context
                .decode_integer("IAFS")
                .ok_or_else(|| Jbig2Error::new("Failed to decode deltaFirstS"))?
        };
        first_s += delta_first_s;
        let mut current_s = first_s;

        loop {
            let mut current_t = 0;
            if strip_size > 1 {
                current_t = if huffman {
                    huffman_input.unwrap().read_bits(log_strip_size)? as i32
                } else {
                    decoding_context
                        .decode_integer("IAIT")
                        .ok_or_else(|| Jbig2Error::new("Failed to decode currentT"))?
                };
            }

            let t = (strip_size as i32) * strip_t + current_t;

            let symbol_id = if huffman {
                match huffman_tables
                    .unwrap()
                    .symbol_id_table
                    .decode(huffman_input.unwrap())?
                {
                    Some(id) => id,
                    None => return Err(Jbig2Error::new("Unexpected OOB in symbolID decode")),
                }
            } else {
                decoding_context.decode_iaid(symbol_code_length) as i32
            };

            let apply_refinement = refinement
                && if huffman {
                    huffman_input.unwrap().read_bit()? != 0
                } else {
                    decoding_context
                        .decode_integer("IARI")
                        .ok_or_else(|| Jbig2Error::new("Failed to decode refinement flag"))?
                        != 0
                };

            // Some PDFs in the corpus crash here for some reason.
            let Some(mut symbol_bitmap) = input_symbols.get(symbol_id as usize) else {
                continue;
            };
            let mut symbol_width = symbol_bitmap[0].len();
            let mut symbol_height = symbol_bitmap.len();
            let refined_bitmap_storage: Option<Bitmap>;

            if apply_refinement {
                let rdw = decoding_context
                    .decode_integer("IARDW")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode rdw"))?;
                let rdh = decoding_context
                    .decode_integer("IARDH")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode rdh"))?;
                let rdx = decoding_context
                    .decode_integer("IARDX")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode rdx"))?;
                let rdy = decoding_context
                    .decode_integer("IARDY")
                    .ok_or_else(|| Jbig2Error::new("Failed to decode rdy"))?;

                symbol_width = (symbol_width as i32 + rdw) as usize;
                symbol_height = (symbol_height as i32 + rdh) as usize;

                let refined_bitmap = decode_refinement(
                    symbol_width,
                    symbol_height,
                    refinement_template_index,
                    symbol_bitmap,
                    (rdw >> 1) + rdx,
                    (rdh >> 1) + rdy,
                    false,
                    refinement_at,
                    decoding_context,
                )?;
                refined_bitmap_storage = Some(refined_bitmap);
                symbol_bitmap = refined_bitmap_storage.as_ref().unwrap();
            }

            let mut increment = 0;
            if !transposed {
                if reference_corner > 1 {
                    current_s += symbol_width as i32 - 1;
                } else {
                    increment = symbol_width as i32 - 1;
                }
            } else if (reference_corner & 1) == 0 {
                current_s += symbol_height as i32 - 1;
            } else {
                increment = symbol_height as i32 - 1;
            }

            let offset_t = t - if (reference_corner & 1) != 0 {
                0
            } else {
                symbol_height as i32 - 1
            };
            let offset_s = current_s
                - if (reference_corner & 2) != 0 {
                    symbol_width as i32 - 1
                } else {
                    0
                };

            if transposed {
                // Place Symbol Bitmap from T1,S1
                for s2 in 0..symbol_height {
                    let row_idx = (offset_s + s2 as i32) as usize;
                    if row_idx >= bitmap.len() {
                        continue;
                    }

                    let symbol_row = &symbol_bitmap[s2];
                    // To ignore Parts of Symbol bitmap which goes outside bitmap region
                    let max_width = ((width as i32) - offset_t).min(symbol_width as i32) as usize;

                    match combination_operator {
                        0 => {
                            // OR
                            for t2 in 0..max_width {
                                let col_idx = (offset_t + t2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() {
                                    bitmap[row_idx][col_idx] |= symbol_row[t2];
                                }
                            }
                        }
                        2 => {
                            // XOR
                            for t2 in 0..max_width {
                                let col_idx = (offset_t + t2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() {
                                    bitmap[row_idx][col_idx] ^= symbol_row[t2];
                                }
                            }
                        }
                        _ => {
                            return Err(Jbig2Error::new(&format!(
                                "operator {combination_operator} is not supported"
                            )));
                        }
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
                        0 => {
                            // OR
                            for s2 in 0..symbol_width {
                                let col_idx = (offset_s + s2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() {
                                    bitmap[row_idx][col_idx] |= symbol_row[s2];
                                }
                            }
                        }
                        2 => {
                            // XOR
                            for s2 in 0..symbol_width {
                                let col_idx = (offset_s + s2 as i32) as usize;
                                if col_idx < bitmap[row_idx].len() {
                                    bitmap[row_idx][col_idx] ^= symbol_row[s2];
                                }
                            }
                        }
                        _ => {
                            return Err(Jbig2Error::new(&format!(
                                "operator {combination_operator} is not supported"
                            )));
                        }
                    }
                }
            }

            i += 1;
            let delta_s = if huffman {
                huffman_tables
                    .unwrap()
                    .table_delta_s
                    .decode(huffman_input.unwrap())?
            } else {
                decoding_context.decode_integer("IADS")
            };

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
    let mut at = Vec::new();
    if !mmr {
        at.push(TemplatePixel {
            x: -(pattern_width as i32),
            y: 0,
        });
        if template == 0 {
            at.push(TemplatePixel { x: -3, y: -1 });
            at.push(TemplatePixel { x: 2, y: -2 });
            at.push(TemplatePixel { x: -2, y: -2 });
        }
    }

    let collective_width = (max_pattern_index + 1) * pattern_width;
    let collective_bitmap = decode_bitmap(
        mmr,
        collective_width,
        pattern_height,
        template,
        false,
        None,
        &at,
        decoding_context,
    )?;

    // Divide collective bitmap into patterns.
    let mut patterns = Vec::new();
    for i in 0..=max_pattern_index {
        let mut pattern_bitmap = Vec::new();
        let x_min = pattern_width * i;
        let x_max = x_min + pattern_width;

        for y in 0..pattern_height {
            pattern_bitmap.push(collective_bitmap[y][x_min..x_max].to_vec());
        }
        patterns.push(pattern_bitmap);
    }

    Ok(patterns)
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
    if enable_skip {
        return Err(Jbig2Error::new("skip is not supported"));
    }
    if combination_operator != 0 {
        return Err(Jbig2Error::new(&format!(
            "operator \"{combination_operator}\" is not supported in halftone region"
        )));
    }

    // Prepare bitmap
    let mut region_bitmap: Vec<Vec<u8>> = Vec::with_capacity(region_height);
    for _ in 0..region_height {
        let mut row = vec![0_u8; region_width];
        if default_pixel_value != 0 {
            row.fill(default_pixel_value);
        }
        region_bitmap.push(row);
    }

    let number_of_patterns = patterns.len();

    let pattern0 = &patterns[0];
    let pattern_width = pattern0[0].len();
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
    let decoding_data = decoding_context.data.clone();

    let mmr_input = if mmr {
        Some(Reader::new(
            &decoding_data,
            decoding_context.start,
            decoding_context.end,
        ))
    } else {
        None
    };

    for _i in 0..bits_per_value {
        let bitmap = if mmr {
            // MMR bit planes are in one continuous stream. Only EOFB codes indicate
            // the end of each bitmap, so EOFBs must be decoded.
            decode_mmr_bitmap(
                mmr_input.as_ref().unwrap(),
                grid_width,
                grid_height,
                true, // end_of_block = true for bit planes
            )?
        } else {
            decode_bitmap(
                false,
                grid_width,
                grid_height,
                template,
                false,
                None,
                &at,
                decoding_context,
            )?
        };
        gray_scale_bit_planes.push(bitmap);
    }

    gray_scale_bit_planes.reverse();

    // 6.6.5.2 Rendering the patterns
    for mg in 0..grid_height {
        for ng in 0..grid_width {
            let mut bit = 0_u8;
            let mut pattern_index = 0_usize;

            // Gray decoding - extract pattern index from bit planes
            for j in (0..bits_per_value).rev() {
                bit ^= gray_scale_bit_planes[j][mg][ng]; // Gray decoding
                pattern_index |= (bit as usize) << j;
            }

            let pattern_bitmap = &patterns[pattern_index];

            let x =
                (grid_offset_x + (mg as i32) * grid_vector_y + (ng as i32) * grid_vector_x) >> 8;
            let y =
                (grid_offset_y + (mg as i32) * grid_vector_x - (ng as i32) * grid_vector_y) >> 8;

            // Draw pattern bitmap at (x, y)
            if x >= 0
                && x + (pattern_width as i32) <= region_width as i32
                && y >= 0
                && y + (pattern_height as i32) <= region_height as i32
            {
                for i in 0..pattern_height {
                    let region_y = (y + i as i32) as usize;
                    let pattern_row = &pattern_bitmap[i];
                    let region_row = &mut region_bitmap[region_y];
                    for j in 0..pattern_width {
                        let region_x = (x + j as i32) as usize;
                        region_row[region_x] |= pattern_row[j];
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
                        if region_x >= 0 && (region_x as usize) < region_width {
                            region_row[region_x as usize] |= pattern_row[j];
                        }
                    }
                }
            }
        }
    }

    Ok(region_bitmap)
}

// Segment header reading - ported from readSegmentHeader function
fn read_segment_header(data: &[u8], start: usize) -> Result<SegmentHeader, Jbig2Error> {
    let number = read_uint32(data, start);
    let flags = data[start + 4];
    let segment_type = flags & 0x3f;

    if segment_type as usize >= SEGMENT_TYPES.len()
        || SEGMENT_TYPES[segment_type as usize].is_none()
    {
        return Err(Jbig2Error::new(&format!(
            "invalid segment type: {segment_type}"
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
    if length == 0xffffffff {
        // 7.2.7 Segment data length, unknown segment length
        if segment_type == 38 {
            // ImmediateGenericRegion
            let generic_region_info = read_region_segment_information(data, position)?;
            let region_segment_information_field_length = 17;
            let generic_region_segment_flags =
                data[position + region_segment_information_field_length];
            let generic_region_mmr = (generic_region_segment_flags & 1) != 0;

            // Searching for the segment end
            let search_pattern_length = 6;
            let mut search_pattern = vec![0_u8; search_pattern_length];
            if !generic_region_mmr {
                search_pattern[0] = 0xff;
                search_pattern[1] = 0xac;
            }
            search_pattern[2] = (generic_region_info.height >> 24) as u8;
            search_pattern[3] = (generic_region_info.height >> 16) as u8;
            search_pattern[4] = (generic_region_info.height >> 8) as u8;
            search_pattern[5] = generic_region_info.height as u8;

            let mut found_length = None;
            for i in position..data.len() {
                let mut j = 0;
                while j < search_pattern_length && search_pattern[j] == data[i + j] {
                    j += 1;
                }
                if j == search_pattern_length {
                    found_length = Some(i + search_pattern_length);
                    break;
                }
            }

            let actual_length =
                found_length.ok_or_else(|| Jbig2Error::new("segment end was not found"))?;

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
        } else {
            return Err(Jbig2Error::new("invalid unknown segment length"));
        }
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

fn get_standard_table(number: u32) -> Result<HuffmanTable, Jbig2Error> {
    if number == 0 || number > 15 {
        Err(Jbig2Error::new("invalid standard table"))
    } else {
        Ok(LazyLock::force(&STANDARD_TABLES[number as usize - 1]).clone())
    }
}

static STANDARD_TABLES: [LazyLock<HuffmanTable>; 15] = [
    LazyLock::new(|| build_standard_table(1)),
    LazyLock::new(|| build_standard_table(2)),
    LazyLock::new(|| build_standard_table(3)),
    LazyLock::new(|| build_standard_table(4)),
    LazyLock::new(|| build_standard_table(5)),
    LazyLock::new(|| build_standard_table(6)),
    LazyLock::new(|| build_standard_table(7)),
    LazyLock::new(|| build_standard_table(8)),
    LazyLock::new(|| build_standard_table(9)),
    LazyLock::new(|| build_standard_table(10)),
    LazyLock::new(|| build_standard_table(11)),
    LazyLock::new(|| build_standard_table(12)),
    LazyLock::new(|| build_standard_table(13)),
    LazyLock::new(|| build_standard_table(14)),
    LazyLock::new(|| build_standard_table(15)),
];

fn build_standard_table(number: u32) -> HuffmanTable {
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
            vec![6, 0x3f],         // OOB
        ],
        3 => vec![
            vec![-256, 8, 8, 0xfe],
            vec![0, 1, 0, 0x0],
            vec![1, 2, 0, 0x2],
            vec![2, 3, 0, 0x6],
            vec![3, 4, 3, 0xe],
            vec![11, 5, 6, 0x1e],
            vec![-257, 8, 32, 0xff, -1], // lower (using -1 as marker)
            vec![75, 7, 32, 0x7e],       // upper
            vec![6, 0x3e],               // OOB
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
            vec![76, 6, 32, 0x3e],       // upper
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
            vec![2048, 6, 32, 0x3f],      // upper
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
            vec![2048, 5, 32, 0x1f],      // upper
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
            vec![1670, 9, 32, 0x1ff],    // upper
            vec![2, 0x1],                // OOB
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
            vec![3339, 9, 32, 0x1ff],    // upper
            vec![2, 0x0],                // OOB
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
            vec![4166, 8, 32, 0xff],    // upper
            vec![2, 0x2],               // OOB
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
            vec![25, 7, 32, 0x7f],      // upper
        ],
        _ => unreachable!(),
    };

    // Convert to HuffmanLine objects using unified constructor
    let mut lines = Vec::new();
    for line_data in lines_data {
        lines.push(HuffmanLine::new(&line_data));
    }

    HuffmanTable::new(lines, true)
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
    _start: usize,
    end: usize,
    position: usize,
    shift: i32,
    current_byte: u8,
}

#[derive(Debug)]
struct Reader<'a>(RefCell<ReaderInner<'a>>);

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], start: usize, end: usize) -> Self {
        Self(RefCell::new(ReaderInner {
            data,
            end,
            _start: start,
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
        let mut result = 0_u32;
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

#[derive(Debug)]
struct TextRegionHuffmanTables {
    symbol_id_table: HuffmanTable,
    table_delta_t: HuffmanTable,
    table_delta_s: HuffmanTable,
    table_first_s: Option<HuffmanTable>,
    _ds_table: Option<HuffmanTable>,
    _dt_table: Option<HuffmanTable>,
    _rdw_table: Option<HuffmanTable>,
    _rdh_table: Option<HuffmanTable>,
    _rdx_table: Option<HuffmanTable>,
    _rdy_table: Option<HuffmanTable>,
    _rsize_table: Option<HuffmanTable>,
}

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
            self.children[bit] = Some(Box::new(Self::new_leaf(line)));
        } else {
            // Create an intermediate node and continue recursively
            if self.children[bit].is_none() {
                self.children[bit] = Some(Box::new(Self::new_node()));
            }
            self.children[bit]
                .as_mut()
                .unwrap()
                .build_tree(line, shift - 1);
        }
    }

    fn decode_node(&self, reader: &Reader<'_>) -> Result<Option<i32>, Jbig2Error> {
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

    fn print_node(&self, indent: usize, path: &str) {
        let indent_str = "  ".repeat(indent);

        if self.is_leaf {
            if self.is_oob {
                println!("{indent_str}[{path}] LEAF: OOB");
            } else {
                println!(
                    "{}[{}] LEAF: rangeLow={}, rangeLength={}, isLowerRange={}",
                    indent_str, path, self.range_low, self.range_length, self.is_lower_range
                );
            }
        } else {
            println!("{indent_str}[{path}] NODE");

            if let Some(ref child) = self.children[0] {
                child.print_node(indent + 1, &format!("{path}0"));
            }

            if let Some(ref child) = self.children[1] {
                child.print_node(indent + 1, &format!("{path}1"));
            }
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

        // table.print_tree();
        Self { root_node }
    }

    fn decode(&self, reader: &Reader<'_>) -> Result<Option<i32>, Jbig2Error> {
        self.root_node.decode_node(reader)
    }

    // For debugging purposes.
    #[allow(dead_code)]
    fn print_tree(&self) {
        println!("=== Huffman Tree Structure ===");
        self.root_node.print_node(0, "ROOT");
        println!("=============================");
    }

    fn assign_prefix_codes(lines: &mut [HuffmanLine]) {
        // Annex B.3 Assigning the prefix codes
        let mut prefix_length_max = 0_usize;
        for line in lines.iter() {
            prefix_length_max = prefix_length_max.max(line.prefix_length);
        }

        let mut histogram = vec![0_u32; prefix_length_max + 1];
        for line in lines.iter() {
            histogram[line.prefix_length] += 1;
        }

        let mut current_length = 1_usize;
        let mut first_code = 0_u32;
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
    data: Rc<Vec<u8>>,
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
                warn!("Error parsing JBIG2 chunk: {e}");
                return None;
            }
        }

        // Process segments with visitor pattern to generate final bitmap
        let mut visitor = SimpleSegmentVisitor::new();

        if let Err(e) = process_segments(&self.segments, &mut visitor) {
            warn!("Error processing JBIG2 segments: {e}");
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
        let data = &chunk.data;
        let mut position = chunk.start;
        let end = chunk.end;

        // Parse file header if present (first 9 bytes for file organization)
        let header = if position + 9 <= data.len()
            && data[position..position + 4] == [0x97, 0x4A, 0x42, 0x32]
        {
            // Read file header flags
            let file_organization_flags = data[position + 8];
            let random_access = (file_organization_flags & 1) != 0;
            position += 9; // Skip the file header
            Jbig2Header { random_access }
        } else {
            Jbig2Header::default()
        };

        // Read segments using extracted function like JavaScript implementation
        let segments = read_segments(&header, data, position, end)?;

        for segment in segments {
            self.segments.push(segment);
        }

        Ok(())
    }
}

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
    _resolution_x: u32,
    _resolution_y: u32,
    _lossless: bool,
    _refinement: bool,
    default_pixel_value: u8,
    combination_operator: u8,
    _requires_buffer: bool,
    combination_operator_override: bool,
}

impl SimpleSegmentVisitor {
    fn new() -> Self {
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
        let mut buffer = vec![0_u8; (row_size * info.height) as usize];

        // Fill with 0xFF if default pixel value is set
        if info.default_pixel_value != 0 {
            buffer.fill(0xff);
        }
        self.buffer = Some(buffer);
    }

    fn draw_bitmap(
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

        let mask0 = 128_u8 >> (region_info.x & 7);
        let mut offset0 = (region_info.y * row_size as u32 + (region_info.x >> 3)) as usize;

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
                        if j < bitmap[i].len() && bitmap[i][j] != 0 && offset < buffer.len() {
                            buffer[offset] |= mask;
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
                        if j < bitmap[i].len() && bitmap[i][j] != 0 && offset < buffer.len() {
                            buffer[offset] ^= mask;
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
                    "operator {combination_operator} is not supported"
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

        let bitmap = decode_bitmap(
            region.mmr, // mmr
            region.info.width as usize,
            region.info.height as usize,
            region.template,
            region.prediction,
            None, // skip
            &region.at,
            &mut decoding_context,
        )?;

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
        let (huffman_tables, huffman_input) = if dictionary.huffman {
            (
                Some(self.get_symbol_dictionary_huffman_tables(dictionary, referred_segments)?),
                Some(Reader::new(data, start, end)),
            )
        } else {
            (None, None)
        };

        let symbols = &mut self.symbols;

        // Collect input symbols from referred segments
        let mut input_symbols = Vec::new();
        for &referred_segment in referred_segments {
            if let Some(referred_symbols) = symbols.get(&referred_segment) {
                input_symbols.extend(referred_symbols.iter().cloned());
            }
        }

        let mut decoding_context = DecodingContext::new(data.to_vec(), start, end);
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
            huffman_input.as_ref(),
        )?;

        if let Some(entry) = symbols.get_mut(&current_segment) {
            entry.extend(new_symbols);
        } else {
            symbols.insert(current_segment, new_symbols);
        }

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

        let (huffman_input, huffman_table) = if region.huffman {
            let huffman_input = Reader::new(data, start, end);
            let huffman_table = self.get_text_region_huffman_tables(
                region,
                referred_segments,
                input_symbols.len(),
                Some(&huffman_input),
            )?;

            (Some(huffman_input), Some(huffman_table))
        } else {
            (None, None)
        };

        let mut decoding_context = DecodingContext::new(data.to_vec(), start, end);
        let symbol_code_length = log2(input_symbols.len()).max(1);

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
            huffman_table.as_ref(),
            region.refinement_template,
            &region.refinement_at,
            &mut decoding_context,
            region.log_strip_size,
            huffman_input.as_ref(),
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
        let patterns = self.patterns[&referred_segments[0]].clone();

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
        number_of_symbols: usize,
        huffman_reader: Option<&Reader<'_>>,
    ) -> Result<TextRegionHuffmanTables, Jbig2Error> {
        // 7.4.3.1.6 Text region segment Huffman table selection
        let mut custom_index = 0;

        // Symbol ID table decoding
        let symbol_id_table = if let Some(reader) = huffman_reader {
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
            0..=2 => get_standard_table(region.huffman_ds as u32 + 8)?,
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
            0..=2 => get_standard_table(region.huffman_dt as u32 + 11)?,
            3 => {
                // custom_index += 1;
                get_custom_huffman_table(custom_index, referred_segments, &self.custom_tables)?
                    .clone()
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
            table_delta_t: t_table.clone(),
            table_delta_s: s_table.clone(),
            table_first_s: fs_table,
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
    _bitmap_coding_context_used: bool,
    _bitmap_coding_context_retained: bool,
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
    _huffman_refinement_dw: u8,
    _huffman_refinement_dh: u8,
    _huffman_refinement_dx: u8,
    _huffman_refinement_dy: u8,
    _huffman_refinement_size_selector: bool,
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

fn decode_mmr_bitmap(
    reader: &Reader<'_>,
    width: usize,
    height: usize,
    end_of_block: bool,
) -> Result<Bitmap, Jbig2Error> {
    let params = CCITTFaxDecoderOptions {
        k: -1,
        columns: width,
        rows: height,
        black_is_1: true,
        eoblock: end_of_block,
        ..Default::default()
    };

    let mut borrowed = reader.0.borrow_mut();

    let mut reader = CrateReader::new_with(&borrowed.data[borrowed.position..borrowed.end], 0);
    let mut decoder = CCITTFaxDecoder::new(&mut reader, params);
    let mut bitmap = Vec::with_capacity(height);
    let mut eof = false;

    for _ in 0..height {
        let row = Rc::new(RefCell::new(vec![]));
        bitmap.push(row.clone());
        let mut shift = -1_i32;
        let mut current_byte = 0_u8;

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
            row.borrow_mut().push(bit);
            shift -= 1;
        }
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

    borrowed.position += decoder.source().offset();

    Ok(bitmap.into_iter().map(|i| i.borrow().clone()).collect())
}

fn read_uncompressed_bitmap(
    reader: &Reader<'_>,
    width: usize,
    height: usize,
) -> Result<Bitmap, Jbig2Error> {
    let mut bitmap = Vec::new();

    for _ in 0..height {
        let row = Rc::new(RefCell::new(Vec::with_capacity(width)));
        bitmap.push(row.clone());

        for _ in 0..width {
            let bit = reader.read_bit()?;
            row.borrow_mut().push(bit);
        }
        reader.byte_align();
    }

    Ok(bitmap.into_iter().map(|i| i.borrow().clone()).collect())
}

fn process_segment(
    segment: &Segment,
    visitor: &mut SimpleSegmentVisitor,
) -> Result<(), Jbig2Error> {
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

            position += 2;

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
                _bitmap_coding_context_used: bitmap_coding_context_used,
                _bitmap_coding_context_retained: bitmap_coding_context_retained,
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
            let strip_size = 1_u32 << log_strip_size;
            let reference_corner = ((text_region_segment_flags >> 4) & 3) as u8;
            let transposed = (text_region_segment_flags & 64) != 0;
            let combination_operator = ((text_region_segment_flags >> 7) & 3) as u8;
            let default_pixel_value = ((text_region_segment_flags >> 9) & 1) as u8;
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
            let mut huffman_fs = 0_u8;
            let mut huffman_ds = 0_u8;
            let mut huffman_dt = 0_u8;
            let mut huffman_refinement_dw = 0_u8;
            let mut huffman_refinement_dh = 0_u8;
            let mut huffman_refinement_dx = 0_u8;
            let mut huffman_refinement_dy = 0_u8;
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
                _huffman_refinement_dw: huffman_refinement_dw,
                _huffman_refinement_dh: huffman_refinement_dh,
                _huffman_refinement_dx: huffman_refinement_dx,
                _huffman_refinement_dy: huffman_refinement_dy,
                _huffman_refinement_size_selector: huffman_refinement_size_selector,
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
            let combination_operator = (halftone_region_flags >> 4) & 7;
            let default_pixel_value = (halftone_region_flags >> 7) & 1;

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
            let resolution_x = read_uint32(data, position + 8);
            let resolution_y = read_uint32(data, position + 12);
            let page_segment_flags = data[position + 16];
            let _page_striping_information = read_uint16(data, position + 17); // Read but not stored, like JS

            // Extract all flags from pageSegmentFlags exactly like JavaScript
            let lossless = (page_segment_flags & 1) != 0;
            let refinement = (page_segment_flags & 2) != 0;
            let default_pixel_value = (page_segment_flags >> 2) & 1;
            let combination_operator = (page_segment_flags >> 3) & 3;
            let requires_buffer = (page_segment_flags & 32) != 0;
            let combination_operator_override = (page_segment_flags & 64) != 0;

            let page_info = PageInfo {
                width,
                height: if height == 0xffffffff { 0 } else { height }, // Handle unknown height like JS
                _resolution_x: resolution_x,
                _resolution_y: resolution_y,
                _lossless: lossless,
                _refinement: refinement,
                default_pixel_value,
                combination_operator,
                _requires_buffer: requires_buffer,
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

fn process_segments(
    segments: &[Segment],
    visitor: &mut SimpleSegmentVisitor,
) -> Result<(), Jbig2Error> {
    for segment in segments {
        process_segment(segment, visitor)?;
    }
    Ok(())
}

fn decode_symbol_id_huffman_table(
    reader: &Reader<'_>,
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
    let run_codes_table = HuffmanTable::new(codes.clone(), false);

    codes.truncate(0);
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
                    let prev_length = codes[i - 1].prefix_length as i32;
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
                codes.push(HuffmanLine::new(&[i as i32, repeated_length, 0, 0]));
                i += 1;
            }
        } else {
            codes.push(HuffmanLine::new(&[i as i32, code_length, 0, 0]));
            i += 1;
        }
    }

    reader.byte_align();
    Ok(HuffmanTable::new(codes, false))
}

/// Tables segment decoding - ported from decodeTablesSegment function
fn decode_tables_segment(
    data: &[u8],
    start: usize,
    end: usize,
) -> Result<HuffmanTable, Jbig2Error> {
    if start + 9 > data.len() {
        return Err(Jbig2Error::new("insufficient data for tables segment"));
    }

    let flags = data[start];
    let lowest_value = read_uint32(data, start + 1) as i32;
    let highest_value = read_uint32(data, start + 5) as i32;
    let reader = Reader::new(data, start + 9, end);

    let prefix_size_bits = ((flags >> 1) & 7) + 1;
    let range_size_bits = ((flags >> 4) & 7) + 1;

    let mut lines = Vec::new();
    let mut current_range_low = lowest_value;

    // Normal table lines
    loop {
        let prefix_length = reader.read_bits(prefix_size_bits as usize)? as i32;
        let range_length = reader.read_bits(range_size_bits as usize)? as i32;

        lines.push(HuffmanLine::new(&[
            current_range_low,
            prefix_length,
            range_length,
            0,
        ]));

        current_range_low += 1 << range_length;

        if current_range_low >= highest_value {
            break;
        }
    }

    // Lower range table line
    let prefix_length = reader.read_bits(prefix_size_bits as usize)? as i32;
    lines.push(HuffmanLine::new(&[
        lowest_value - 1,
        prefix_length,
        32,
        0,
        -1, // "lower" marker
    ]));

    // Upper range table line
    let prefix_length = reader.read_bits(prefix_size_bits as usize)? as i32;
    lines.push(HuffmanLine::new(&[highest_value, prefix_length, 32, 0]));

    // Out-of-band table line
    if (flags & 1) != 0 {
        let prefix_length = reader.read_bits(prefix_size_bits as usize)? as i32;
        lines.push(HuffmanLine::new(&[prefix_length, 0]));
    }

    Ok(HuffmanTable::new(lines, false))
}

/// Custom Huffman table getter - ported from getCustomHuffmanTable function  
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

// Just for debugging purposes.
#[allow(dead_code)]
fn print_bitmap(entries: &Vec<Vec<u8>>) {
    for e in entries {
        for b in e {
            print!("{b}");
        }
        println!();
    }
}
