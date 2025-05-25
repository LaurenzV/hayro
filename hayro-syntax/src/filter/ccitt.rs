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
/* Copyright 1996-2003 Glyph & Cog, LLC
 *
 * The CCITT stream implementation contained in this file is a JavaScript port
 * of XPDF's implementation, made available under the Apache 2.0 open source
 * license.
 */

//! A decoder for CCITT streams, translated from <https://github.com/mozilla/pdf.js/blob/master/src/core/ccitt.js>

use crate::object::dict::Dict;
use crate::object::dict::keys::{
    BLACK_IS_1, COLUMNS, ENCODED_BYTE_ALIGN, END_OF_BLOCK, END_OF_LINE, K, ROWS,
};
use crate::reader::Reader;
use log::warn;

/// Decode a CCITT data stream.
pub fn decode(data: &[u8], params: Dict) -> Option<Vec<u8>> {
    let dp = CCITTFaxDecoderOptions::default();

    let params = CCITTFaxDecoderOptions {
        k: params.get::<i32>(K).unwrap_or(dp.k),
        end_of_line: params.get::<bool>(END_OF_LINE).unwrap_or(dp.end_of_line),
        encoded_byte_align: params
            .get::<bool>(ENCODED_BYTE_ALIGN)
            .unwrap_or(dp.encoded_byte_align),
        columns: params.get::<usize>(COLUMNS).unwrap_or(dp.columns),
        rows: params.get::<usize>(ROWS).unwrap_or(dp.rows),
        eoblock: params.get::<bool>(END_OF_BLOCK).unwrap_or(dp.eoblock),
        black_is_1: params.get::<bool>(BLACK_IS_1).unwrap_or(dp.black_is_1),
    };

    let mut reader = Reader::new(data);
    let mut decoder = CCITTFaxDecoder::new(&mut reader, params);
    let mut out = vec![];

    loop {
        let byte = decoder.read_next_char();
        if byte == -1 {
            break;
        }

        out.push(byte as u8);
    }

    Some(out)
}

const CCITT_EOL: i32 = -2;
const CCITT_EOF: i32 = -1;
const TWO_DIM_PASS: i32 = 0;
const TWO_DIM_HORIZ: i32 = 1;
const TWO_DIM_VERT_0: i32 = 2;
const TWO_DIM_VERT_R1: i32 = 3;
const TWO_DIM_VERT_L1: i32 = 4;
const TWO_DIM_VERT_R2: i32 = 5;
const TWO_DIM_VERT_L2: i32 = 6;
const TWO_DIM_VERT_R3: i32 = 7;
const TWO_DIM_VERT_L3: i32 = 8;

const TWO_DIM_TABLE: [[i32; 2]; 128] = [
    [-1, -1],
    [-1, -1],
    [7, TWO_DIM_VERT_L3],
    [7, TWO_DIM_VERT_R3],
    [6, TWO_DIM_VERT_L2],
    [6, TWO_DIM_VERT_L2],
    [6, TWO_DIM_VERT_R2],
    [6, TWO_DIM_VERT_R2],
    [4, TWO_DIM_PASS],
    [4, TWO_DIM_PASS],
    [4, TWO_DIM_PASS],
    [4, TWO_DIM_PASS],
    [4, TWO_DIM_PASS],
    [4, TWO_DIM_PASS],
    [4, TWO_DIM_PASS],
    [4, TWO_DIM_PASS],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_HORIZ],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_L1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [3, TWO_DIM_VERT_R1],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
    [1, TWO_DIM_VERT_0],
];

const WHITE_TABLE_1: [[i32; 2]; 32] = [
    [-1, -1],
    [12, CCITT_EOL],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [11, 1792],
    [11, 1792],
    [12, 1984],
    [12, 2048],
    [12, 2112],
    [12, 2176],
    [12, 2240],
    [12, 2304],
    [11, 1856],
    [11, 1856],
    [11, 1920],
    [11, 1920],
    [12, 2368],
    [12, 2432],
    [12, 2496],
    [12, 2560],
];

const WHITE_TABLE_2: [[i32; 2]; 512] = [
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [8, 29],
    [8, 29],
    [8, 30],
    [8, 30],
    [8, 45],
    [8, 45],
    [8, 46],
    [8, 46],
    [7, 22],
    [7, 22],
    [7, 22],
    [7, 22],
    [7, 23],
    [7, 23],
    [7, 23],
    [7, 23],
    [8, 47],
    [8, 47],
    [8, 48],
    [8, 48],
    [6, 13],
    [6, 13],
    [6, 13],
    [6, 13],
    [6, 13],
    [6, 13],
    [6, 13],
    [6, 13],
    [7, 20],
    [7, 20],
    [7, 20],
    [7, 20],
    [8, 33],
    [8, 33],
    [8, 34],
    [8, 34],
    [8, 35],
    [8, 35],
    [8, 36],
    [8, 36],
    [8, 37],
    [8, 37],
    [8, 38],
    [8, 38],
    [7, 19],
    [7, 19],
    [7, 19],
    [7, 19],
    [8, 31],
    [8, 31],
    [8, 32],
    [8, 32],
    [6, 1],
    [6, 1],
    [6, 1],
    [6, 1],
    [6, 1],
    [6, 1],
    [6, 1],
    [6, 1],
    [6, 12],
    [6, 12],
    [6, 12],
    [6, 12],
    [6, 12],
    [6, 12],
    [6, 12],
    [6, 12],
    [8, 53],
    [8, 53],
    [8, 54],
    [8, 54],
    [7, 26],
    [7, 26],
    [7, 26],
    [7, 26],
    [8, 39],
    [8, 39],
    [8, 40],
    [8, 40],
    [8, 41],
    [8, 41],
    [8, 42],
    [8, 42],
    [8, 43],
    [8, 43],
    [8, 44],
    [8, 44],
    [7, 21],
    [7, 21],
    [7, 21],
    [7, 21],
    [7, 28],
    [7, 28],
    [7, 28],
    [7, 28],
    [8, 61],
    [8, 61],
    [8, 62],
    [8, 62],
    [8, 63],
    [8, 63],
    [8, 0],
    [8, 0],
    [8, 320],
    [8, 320],
    [8, 384],
    [8, 384],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 10],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [5, 11],
    [7, 27],
    [7, 27],
    [7, 27],
    [7, 27],
    [8, 59],
    [8, 59],
    [8, 60],
    [8, 60],
    [9, 1472],
    [9, 1536],
    [9, 1600],
    [9, 1728],
    [7, 18],
    [7, 18],
    [7, 18],
    [7, 18],
    [7, 24],
    [7, 24],
    [7, 24],
    [7, 24],
    [8, 49],
    [8, 49],
    [8, 50],
    [8, 50],
    [8, 51],
    [8, 51],
    [8, 52],
    [8, 52],
    [7, 25],
    [7, 25],
    [7, 25],
    [7, 25],
    [8, 55],
    [8, 55],
    [8, 56],
    [8, 56],
    [8, 57],
    [8, 57],
    [8, 58],
    [8, 58],
    [6, 192],
    [6, 192],
    [6, 192],
    [6, 192],
    [6, 192],
    [6, 192],
    [6, 192],
    [6, 192],
    [6, 1664],
    [6, 1664],
    [6, 1664],
    [6, 1664],
    [6, 1664],
    [6, 1664],
    [6, 1664],
    [6, 1664],
    [8, 448],
    [8, 448],
    [8, 512],
    [8, 512],
    [9, 704],
    [9, 768],
    [8, 640],
    [8, 640],
    [8, 576],
    [8, 576],
    [9, 832],
    [9, 896],
    [9, 960],
    [9, 1024],
    [9, 1088],
    [9, 1152],
    [9, 1216],
    [9, 1280],
    [9, 1344],
    [9, 1408],
    [7, 256],
    [7, 256],
    [7, 256],
    [7, 256],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 2],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [4, 3],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 128],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 8],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [5, 9],
    [6, 16],
    [6, 16],
    [6, 16],
    [6, 16],
    [6, 16],
    [6, 16],
    [6, 16],
    [6, 16],
    [6, 17],
    [6, 17],
    [6, 17],
    [6, 17],
    [6, 17],
    [6, 17],
    [6, 17],
    [6, 17],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 4],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [6, 14],
    [6, 14],
    [6, 14],
    [6, 14],
    [6, 14],
    [6, 14],
    [6, 14],
    [6, 14],
    [6, 15],
    [6, 15],
    [6, 15],
    [6, 15],
    [6, 15],
    [6, 15],
    [6, 15],
    [6, 15],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [5, 64],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
    [4, 7],
];

const BLACK_TABLE_1: [[i32; 2]; 128] = [
    [-1, -1],
    [-1, -1],
    [12, CCITT_EOL],
    [12, CCITT_EOL],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [11, 1792],
    [11, 1792],
    [11, 1792],
    [11, 1792],
    [12, 1984],
    [12, 1984],
    [12, 2048],
    [12, 2048],
    [12, 2112],
    [12, 2112],
    [12, 2176],
    [12, 2176],
    [12, 2240],
    [12, 2240],
    [12, 2304],
    [12, 2304],
    [11, 1856],
    [11, 1856],
    [11, 1856],
    [11, 1856],
    [11, 1920],
    [11, 1920],
    [11, 1920],
    [11, 1920],
    [12, 2368],
    [12, 2368],
    [12, 2432],
    [12, 2432],
    [12, 2496],
    [12, 2496],
    [12, 2560],
    [12, 2560],
    [10, 18],
    [10, 18],
    [10, 18],
    [10, 18],
    [10, 18],
    [10, 18],
    [10, 18],
    [10, 18],
    [12, 52],
    [12, 52],
    [13, 640],
    [13, 704],
    [13, 768],
    [13, 832],
    [12, 55],
    [12, 55],
    [12, 56],
    [12, 56],
    [13, 1280],
    [13, 1344],
    [13, 1408],
    [13, 1472],
    [12, 59],
    [12, 59],
    [12, 60],
    [12, 60],
    [13, 1536],
    [13, 1600],
    [11, 24],
    [11, 24],
    [11, 24],
    [11, 24],
    [11, 25],
    [11, 25],
    [11, 25],
    [11, 25],
    [13, 1664],
    [13, 1728],
    [12, 320],
    [12, 320],
    [12, 384],
    [12, 384],
    [12, 448],
    [12, 448],
    [13, 512],
    [13, 576],
    [12, 53],
    [12, 53],
    [12, 54],
    [12, 54],
    [13, 896],
    [13, 960],
    [13, 1024],
    [13, 1088],
    [13, 1152],
    [13, 1216],
    [10, 64],
    [10, 64],
    [10, 64],
    [10, 64],
    [10, 64],
    [10, 64],
    [10, 64],
    [10, 64],
];

const BLACK_TABLE_2: [[i32; 2]; 192] = [
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [8, 13],
    [11, 23],
    [11, 23],
    [12, 50],
    [12, 51],
    [12, 44],
    [12, 45],
    [12, 46],
    [12, 47],
    [12, 57],
    [12, 58],
    [12, 61],
    [12, 256],
    [10, 16],
    [10, 16],
    [10, 16],
    [10, 16],
    [10, 17],
    [10, 17],
    [10, 17],
    [10, 17],
    [12, 48],
    [12, 49],
    [12, 62],
    [12, 63],
    [12, 30],
    [12, 31],
    [12, 32],
    [12, 33],
    [12, 40],
    [12, 41],
    [11, 22],
    [11, 22],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [8, 14],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 10],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [7, 11],
    [9, 15],
    [9, 15],
    [9, 15],
    [9, 15],
    [9, 15],
    [9, 15],
    [9, 15],
    [9, 15],
    [12, 128],
    [12, 192],
    [12, 26],
    [12, 27],
    [12, 28],
    [12, 29],
    [11, 19],
    [11, 19],
    [11, 20],
    [11, 20],
    [12, 34],
    [12, 35],
    [12, 36],
    [12, 37],
    [12, 38],
    [12, 39],
    [11, 21],
    [11, 21],
    [12, 42],
    [12, 43],
    [10, 0],
    [10, 0],
    [10, 0],
    [10, 0],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
    [7, 12],
];

static BLACK_TABLE_3: [[i32; 2]; 64] = [
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [-1, -1],
    [6, 9],
    [6, 8],
    [5, 7],
    [5, 7],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 6],
    [4, 5],
    [4, 5],
    [4, 5],
    [4, 5],
    [3, 1],
    [3, 1],
    [3, 1],
    [3, 1],
    [3, 1],
    [3, 1],
    [3, 1],
    [3, 1],
    [3, 4],
    [3, 4],
    [3, 4],
    [3, 4],
    [3, 4],
    [3, 4],
    [3, 4],
    [3, 4],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 3],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
    [2, 2],
];

pub struct CCITTFaxDecoder<'a> {
    pub source: &'a mut Reader<'a>,
    pub eof: bool,
    pub encoding: i32,
    pub eoline: bool,
    pub byte_align: bool,
    pub columns: usize,
    pub rows: usize,
    pub eoblock: bool,
    pub black: bool,
    pub coding_line: Vec<u32>,
    pub ref_line: Vec<u32>,
    pub coding_pos: usize,
    pub row: usize,
    pub next_line_2d: bool,
    pub input_bits: usize,
    pub input_buf: u32,
    pub output_bits: usize,
    pub rows_done: bool,
    pub err: bool,
}

impl<'a> CCITTFaxDecoder<'a> {
    pub fn new(source: &'a mut Reader<'a>, options: CCITTFaxDecoderOptions) -> Self {
        let k = options.k;
        let eoline = options.end_of_line;
        let byte_align = options.encoded_byte_align;
        let columns = options.columns;
        let rows = options.rows;
        let eoblock = options.eoblock;
        let black = options.black_is_1;

        let ref_line = vec![0; columns + 2];
        let mut coding_line = vec![0; columns + 1];
        coding_line[0] = columns as u32;

        let mut decoder = Self {
            source,
            eof: false,
            encoding: k,
            eoline,
            byte_align,
            columns,
            rows,
            eoblock,
            black,
            coding_line,
            ref_line,
            coding_pos: 0,
            row: 0,
            next_line_2d: k < 0,
            input_bits: 0,
            input_buf: 0,
            output_bits: 0,
            rows_done: false,
            err: false,
        };

        let mut code1;
        while {
            code1 = decoder.look_bits(12);
            code1 == 0
        } {
            decoder.eat_bits(1);
        }

        if code1 == 1 {
            decoder.eat_bits(12);
        }

        if decoder.encoding > 0 {
            decoder.next_line_2d = decoder.look_bits(1) == 0;
            decoder.eat_bits(1);
        }

        decoder
    }

    fn look_bits(&mut self, n: usize) -> i32 {
        let mut c;
        while self.input_bits < n {
            if let Some(byte) = self.source.read_byte() {
                c = byte;
            } else {
                if self.input_bits == 0 {
                    return CCITT_EOF;
                }

                return ((self.input_buf << (n - self.input_bits)) & (0xffff >> (16 - n))) as i32;
            }
            self.input_buf = (self.input_buf << 8) | c as u32;
            self.input_bits += 8;
        }
        ((self.input_buf >> (self.input_bits - n)) & (0xffff >> (16 - n))) as i32
    }

    fn eat_bits(&mut self, n: usize) {
        if self.input_bits < n {
            self.input_bits = 0;
        } else {
            self.input_bits -= n;
        }
    }

    fn add_pixels(&mut self, a1: u32, black_pixels: bool) {
        if a1 > self.coding_line[self.coding_pos] {
            if a1 > self.columns as u32 {
                warn!("row is wrong length");

                self.err = true;
            }

            if ((self.coding_pos & 1) != 0) ^ black_pixels {
                self.coding_pos += 1;
            }

            self.coding_line[self.coding_pos] = a1;
        }
    }

    fn add_pixels_neg(&mut self, a1: u32, black_pixels: bool) {
        if a1 > self.coding_line[self.coding_pos] {
            if a1 > self.columns as u32 {
                warn!("row is wrong length");

                self.err = true;
            }

            if ((self.coding_pos & 1) != 0) ^ black_pixels {
                self.coding_pos += 1;
            }

            self.coding_line[self.coding_pos] = a1;
        } else if a1 < self.coding_line[self.coding_pos] {
            // TODO: Investigate why this comparison exists in pdf.js.
            #[allow(unused_comparisons)]
            if a1 < 0 {
                warn!("invalid code");

                self.err = true;
            }

            while self.coding_pos > 0 && a1 < self.coding_line[self.coding_pos - 1] {
                self.coding_pos -= 1;
            }

            self.coding_line[self.coding_pos] = a1;
        }
    }

    fn find_table_code(
        &mut self,
        start: usize,
        end: usize,
        table: &[[i32; 2]],
        limit: Option<usize>,
    ) -> (bool, i32, bool) {
        let limit_value = limit.unwrap_or(0);
        for i in start..=end {
            let code = self.look_bits(i);

            if code == CCITT_EOF {
                return (true, 1, false);
            }

            let mut code_shifted = code;

            if i < end {
                code_shifted <<= end - i;
            }

            if limit_value == 0 || code_shifted as usize >= limit_value {
                let p = table[(code_shifted as usize) - limit_value];
                if p[0] == i as i32 {
                    self.eat_bits(i);
                    return (true, p[1], true);
                }
            }
        }
        (false, 0, false)
    }

    fn get_two_dim_code(&mut self) -> i32 {
        if self.eoblock {
            let code = self.look_bits(7);

            if let Some(p) = TWO_DIM_TABLE.get(code as usize) {
                if p[0] > 0 {
                    self.eat_bits(p[0] as usize);
                    return p[1];
                }
            }
        } else {
            let (found, value, matched) = self.find_table_code(1, 7, &TWO_DIM_TABLE, None);

            if found && matched {
                return value;
            }
        }

        // warn!("bad two dim code");

        CCITT_EOF
    }

    fn get_white_code(&mut self) -> i32 {
        let mut code = 0;

        if self.eoblock {
            code = self.look_bits(12);

            if code == CCITT_EOF {
                return 1;
            }

            let code = code as usize;

            let p = if (code >> 5) == 0 {
                &WHITE_TABLE_1[code]
            } else {
                &WHITE_TABLE_2[code >> 3]
            };

            if p[0] > 0 {
                self.eat_bits(p[0] as usize);
                return p[1];
            }
        } else {
            let result = self.find_table_code(1, 9, &WHITE_TABLE_2, None);

            if result.0 {
                return result.1;
            }

            let result = self.find_table_code(11, 12, &WHITE_TABLE_1, None);

            if result.0 {
                return result.1;
            }
        }

        warn!("bad white code: {}", code);

        self.eat_bits(1);
        1
    }

    fn get_black_code(&mut self) -> i32 {
        let code;

        if self.eoblock {
            code = self.look_bits(13);
            if code == CCITT_EOF {
                return 1;
            }

            let p = if (code >> 7) == 0 {
                BLACK_TABLE_1[code as usize]
            } else if (code >> 9) == 0 && (code >> 7) != 0 {
                let index = ((code >> 1) as isize - 64).max(0) as usize;
                BLACK_TABLE_2[index]
            } else {
                BLACK_TABLE_3[(code >> 7) as usize]
            };

            if p[0] > 0 {
                self.eat_bits(p[0] as usize);
                return p[1];
            }
        } else {
            let result = self.find_table_code(2, 6, &BLACK_TABLE_3, None);
            if result.0 {
                return result.1;
            }

            let result = self.find_table_code(7, 12, &BLACK_TABLE_2, Some(64));
            if result.0 {
                return result.1;
            }

            let result = self.find_table_code(10, 13, &BLACK_TABLE_1, None);
            if result.0 {
                return result.1;
            }
        }

        warn!("bad black code");

        self.eat_bits(1);
        1
    }

    pub fn read_next_char(&mut self) -> i32 {
        if self.eof {
            return -1;
        }

        let columns = self.columns;
        let mut ref_pos;
        let mut black_pixels;
        let mut bits;
        let mut i = 0;

        if self.output_bits == 0 {
            if self.rows_done {
                self.eof = true;
            }

            if self.eof {
                return -1;
            }

            self.err = false;

            let mut code1;
            let mut code2;
            let mut code3;
            if self.next_line_2d {
                loop {
                    if self.coding_line[i] >= columns as u32 {
                        break;
                    }
                    self.ref_line[i] = self.coding_line[i];
                    i += 1;
                }

                self.ref_line[i] = columns as u32;
                self.ref_line[i + 1] = columns as u32;
                self.coding_line[0] = 0;
                self.coding_pos = 0;
                ref_pos = 0;
                black_pixels = false;

                while self.coding_line[self.coding_pos] < columns as u32 {
                    code1 = self.get_two_dim_code();

                    match code1 {
                        x if x == TWO_DIM_PASS => {
                            let next_pos = ref_pos + 1;

                            self.add_pixels(self.ref_line[next_pos], black_pixels);

                            if self.ref_line[next_pos] < columns as u32 {
                                ref_pos += 2;
                            }
                        }
                        x if x == TWO_DIM_HORIZ => {
                            code1 = 0;
                            code2 = 0;

                            if black_pixels {
                                loop {
                                    code3 = self.get_black_code();
                                    code1 += code3;
                                    if code3 < 64 {
                                        break;
                                    }
                                }
                                loop {
                                    code3 = self.get_white_code();
                                    code2 += code3;
                                    if code3 < 64 {
                                        break;
                                    }
                                }
                            } else {
                                loop {
                                    code3 = self.get_white_code();
                                    code1 += code3;
                                    if code3 < 64 {
                                        break;
                                    }
                                }
                                loop {
                                    code3 = self.get_black_code();
                                    code2 += code3;
                                    if code3 < 64 {
                                        break;
                                    }
                                }
                            }

                            self.add_pixels(
                                self.coding_line[self.coding_pos] + code1 as u32,
                                black_pixels,
                            );

                            if self.coding_line[self.coding_pos] < columns as u32 {
                                self.add_pixels(
                                    self.coding_line[self.coding_pos] + code2 as u32,
                                    black_pixels ^ true,
                                );
                            }

                            while self.ref_line[ref_pos] <= self.coding_line[self.coding_pos]
                                && self.ref_line[ref_pos] < columns as u32
                            {
                                ref_pos += 2;
                            }
                        }
                        x if x == TWO_DIM_VERT_R3 => {
                            self.add_pixels(self.ref_line[ref_pos] + 3, black_pixels);

                            black_pixels ^= true;

                            if self.coding_line[self.coding_pos] < columns as u32 {
                                ref_pos += 1;
                                while self.ref_line[ref_pos] <= self.coding_line[self.coding_pos]
                                    && self.ref_line[ref_pos] < columns as u32
                                {
                                    ref_pos += 2;
                                }
                            }
                        }
                        x if x == TWO_DIM_VERT_R2 => {
                            self.add_pixels(self.ref_line[ref_pos] + 2, black_pixels);

                            black_pixels ^= true;

                            if self.coding_line[self.coding_pos] < columns as u32 {
                                ref_pos += 1;

                                while self.ref_line[ref_pos] <= self.coding_line[self.coding_pos]
                                    && self.ref_line[ref_pos] < columns as u32
                                {
                                    ref_pos += 2;
                                }
                            }
                        }
                        x if x == TWO_DIM_VERT_R1 => {
                            self.add_pixels(self.ref_line[ref_pos] + 1, black_pixels);

                            black_pixels ^= true;

                            if self.coding_line[self.coding_pos] < columns as u32 {
                                ref_pos += 1;

                                while self.ref_line[ref_pos] <= self.coding_line[self.coding_pos]
                                    && self.ref_line[ref_pos] < columns as u32
                                {
                                    ref_pos += 2;
                                }
                            }
                        }
                        x if x == TWO_DIM_VERT_0 => {
                            self.add_pixels(self.ref_line[ref_pos], black_pixels);

                            black_pixels ^= true;

                            if self.coding_line[self.coding_pos] < columns as u32 {
                                ref_pos += 1;

                                while self.ref_line[ref_pos] <= self.coding_line[self.coding_pos]
                                    && self.ref_line[ref_pos] < columns as u32
                                {
                                    ref_pos += 2;
                                }
                            }
                        }
                        x if x == TWO_DIM_VERT_L3 => {
                            self.add_pixels_neg(self.ref_line[ref_pos] - 3, black_pixels);
                            black_pixels ^= true;
                            if self.coding_line[self.coding_pos] < columns as u32 {
                                if ref_pos > 0 {
                                    ref_pos -= 1;
                                } else {
                                    ref_pos += 1;
                                }
                                while self.ref_line[ref_pos] <= self.coding_line[self.coding_pos]
                                    && self.ref_line[ref_pos] < columns as u32
                                {
                                    ref_pos += 2;
                                }
                            }
                        }
                        x if x == TWO_DIM_VERT_L2 => {
                            self.add_pixels_neg(self.ref_line[ref_pos] - 2, black_pixels);
                            black_pixels ^= true;
                            if self.coding_line[self.coding_pos] < columns as u32 {
                                if ref_pos > 0 {
                                    ref_pos -= 1;
                                } else {
                                    ref_pos += 1;
                                }
                                while self.ref_line[ref_pos] <= self.coding_line[self.coding_pos]
                                    && self.ref_line[ref_pos] < columns as u32
                                {
                                    ref_pos += 2;
                                }
                            }
                        }
                        x if x == TWO_DIM_VERT_L1 => {
                            self.add_pixels_neg(self.ref_line[ref_pos] - 1, black_pixels);

                            black_pixels ^= true;

                            if self.coding_line[self.coding_pos] < columns as u32 {
                                if ref_pos > 0 {
                                    ref_pos -= 1;
                                } else {
                                    ref_pos += 1;
                                }

                                while self.ref_line[ref_pos] <= self.coding_line[self.coding_pos]
                                    && self.ref_line[ref_pos] < columns as u32
                                {
                                    ref_pos += 2;
                                }
                            }
                        }
                        x if x == CCITT_EOF => {
                            self.add_pixels(columns as u32, false);
                            self.eof = true;
                        }
                        _ => {
                            warn!("bad 2d code");

                            self.add_pixels(columns as u32, false);
                            self.err = true;
                        }
                    }
                }
            } else {
                self.coding_line[0] = 0;
                self.coding_pos = 0;
                black_pixels = false;

                while self.coding_line[self.coding_pos] < columns as u32 {
                    code1 = 0;

                    if black_pixels {
                        loop {
                            code3 = self.get_black_code();
                            code1 += code3;
                            if code3 < 64 {
                                break;
                            }
                        }
                    } else {
                        loop {
                            code3 = self.get_white_code();
                            code1 += code3;
                            if code3 < 64 {
                                break;
                            }
                        }
                    }

                    self.add_pixels(
                        self.coding_line[self.coding_pos] + code1 as u32,
                        black_pixels,
                    );
                    black_pixels ^= true;
                }
            }

            let mut got_eol = false;

            if self.byte_align {
                self.input_bits &= !7;
            }

            if !self.eoblock && self.row == self.rows - 1 {
                self.rows_done = true;
            } else {
                code1 = self.look_bits(12);

                if self.eoline {
                    while code1 != CCITT_EOF && code1 != 1 {
                        self.eat_bits(1);
                        code1 = self.look_bits(12);
                    }
                } else {
                    while code1 == 0 {
                        self.eat_bits(1);
                        code1 = self.look_bits(12);
                    }
                }
                if code1 == 1 {
                    self.eat_bits(12);
                    got_eol = true;
                } else if code1 == CCITT_EOF {
                    self.eof = true;
                }
            }

            if !self.eof && self.encoding > 0 && !self.rows_done {
                self.next_line_2d = self.look_bits(1) == 0;
                self.eat_bits(1);
            }

            if self.eoblock && got_eol && self.byte_align {
                code1 = self.look_bits(12);
                if code1 == 1 {
                    self.eat_bits(12);

                    if self.encoding > 0 {
                        self.look_bits(1);
                        self.eat_bits(1);
                    }

                    if self.encoding >= 0 {
                        for _ in 0..4 {
                            code1 = self.look_bits(12);

                            if code1 != 1 {
                                warn!("bad rtc code: {}", code1);
                            }

                            self.eat_bits(12);

                            if self.encoding > 0 {
                                self.look_bits(1);
                                self.eat_bits(1);
                            }
                        }
                    }
                    self.eof = true;
                }
            } else if self.err && self.eoline {
                loop {
                    code1 = self.look_bits(13);

                    if code1 == CCITT_EOF {
                        self.eof = true;
                        return -1;
                    }

                    if code1 >> 1 == 1 {
                        break;
                    }

                    self.eat_bits(1);
                }

                self.eat_bits(12);

                if self.encoding > 0 {
                    self.eat_bits(1);
                    self.next_line_2d = (code1 & 1) == 0;
                }
            }

            self.output_bits = if self.coding_line[0] > 0 {
                self.coding_pos = 0;
                self.coding_line[0]
            } else {
                self.coding_pos = 1;
                self.coding_line[1]
            } as usize;
            self.row += 1;
        }

        let mut c;
        if self.output_bits >= 8 {
            c = if self.coding_pos & 1 != 0 { 0 } else { 0xff };
            self.output_bits -= 8;
            if self.output_bits == 0 && self.coding_line[self.coding_pos] < columns as u32 {
                self.coding_pos += 1;
                self.output_bits = (self.coding_line[self.coding_pos]
                    - self.coding_line[self.coding_pos - 1])
                    as usize;
            }
        } else {
            bits = 8;
            c = 0;
            loop {
                if self.output_bits > bits {
                    c <<= bits;

                    if self.coding_pos & 1 == 0 {
                        c |= 0xff >> (8 - bits);
                    }

                    self.output_bits -= bits;
                    bits = 0;
                } else {
                    c <<= self.output_bits;

                    if self.coding_pos & 1 == 0 {
                        c |= 0xff >> (8 - self.output_bits);
                    }

                    bits -= self.output_bits;
                    self.output_bits = 0;

                    if self.coding_line[self.coding_pos] < columns as u32 {
                        self.coding_pos += 1;
                        self.output_bits = (self.coding_line[self.coding_pos]
                            - self.coding_line[self.coding_pos - 1])
                            as usize;
                    } else if bits > 0 {
                        c <<= bits;
                        bits = 0;
                    }
                }
                if bits == 0 {
                    break;
                }
            }
        }

        if self.black {
            c ^= 0xff;
        }

        c
    }
}

pub struct CCITTFaxDecoderOptions {
    pub k: i32,
    pub end_of_line: bool,
    pub encoded_byte_align: bool,
    pub columns: usize,
    pub rows: usize,
    pub eoblock: bool,
    pub black_is_1: bool,
}

impl Default for CCITTFaxDecoderOptions {
    fn default() -> Self {
        Self {
            k: 0,
            end_of_line: false,
            encoded_byte_align: false,
            columns: 1728,
            rows: 0,
            eoblock: true,
            black_is_1: false,
        }
    }
}
