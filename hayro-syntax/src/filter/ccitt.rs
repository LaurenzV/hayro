use crate::object::Dict;
use crate::object::dict::keys::{
    BLACK_IS_1, COLUMNS, ENCODED_BYTE_ALIGN, END_OF_BLOCK, END_OF_LINE, K, ROWS,
};
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;
use hayro_ccitt::{DecodeSettings, Decoder, EncodingMode};

pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
) -> Option<FilterResult<'static>> {
    let k = params.get::<i32>(K).unwrap_or(0);

    let columns = params.get::<usize>(COLUMNS).unwrap_or(1728) as u32;
    // The image height takes precedence over `Rows`.
    let rows = if image_params.height > 0 {
        image_params.height
    } else {
        params.get::<u32>(ROWS).unwrap_or(0)
    };
    let output_len = (columns as usize).checked_mul(rows as usize)?;

    let settings = DecodeSettings {
        columns,
        rows,
        end_of_block: params.get::<bool>(END_OF_BLOCK).unwrap_or(true),
        end_of_line: params.get::<bool>(END_OF_LINE).unwrap_or(false),
        rows_are_byte_aligned: params.get::<bool>(ENCODED_BYTE_ALIGN).unwrap_or(false),
        encoding: if k < 0 {
            EncodingMode::Group4
        } else if k == 0 {
            EncodingMode::Group3_1D
        } else {
            EncodingMode::Group3_2D
        },
        strict: false,
        cancel: None,
    };
    let white: u8 = if params.get::<bool>(BLACK_IS_1).unwrap_or(false) {
        0x00
    } else {
        0xFF
    };

    // Expand to 8 bpc unless indexed, for easier RGBA8 conversion.
    let (decoded, bpc, decoded_rows) = if image_params.is_indexed {
        struct BitPackDecoder {
            output: Vec<u8>,
            columns: u32,
            stride: usize,
            white: u8,
            last_byte_mask: u8,
            decoded_rows: u32,
        }

        impl Decoder for BitPackDecoder {
            fn push_row(&mut self, changes: &[u32]) {
                let start = self.decoded_rows as usize * self.stride;
                let end = start + self.stride;

                if self.output.len() < end {
                    self.output.resize(end, self.white);
                }

                let row = &mut self.output[start..end];

                // Rows are prefilled with white, so flipping a span makes it black.
                for (start, end) in black_spans(changes, self.columns) {
                    let (first, last) = (start / 8, (end - 1) / 8);
                    let first_mask = 0xFF_u8 >> (start % 8);
                    let last_mask = 0xFF_u8 << (7 - (end - 1) % 8);

                    if first == last {
                        row[first] ^= first_mask & last_mask;
                    } else {
                        row[first] ^= first_mask;
                        fill_short(&mut row[first + 1..last], !self.white);
                        row[last] ^= last_mask;
                    }
                }

                if let Some(last) = row.last_mut() {
                    *last &= self.last_byte_mask;
                }

                self.decoded_rows += 1;
            }
        }

        let stride = (columns as usize).div_ceil(8);
        let mut decoder = BitPackDecoder {
            output: vec![white; stride.checked_mul(rows as usize)?],
            columns,
            stride,
            white,
            last_byte_mask: 0xFF_u8 << ((8 - columns % 8) % 8),
            decoded_rows: 0,
        };
        let result = hayro_ccitt::decode(data, &mut decoder, &settings);

        // Lenient: return partial output if at least one row decoded (0001763.pdf).
        if result.is_err() && decoder.decoded_rows == 0 {
            return None;
        }

        decoder
            .output
            .truncate(decoder.decoded_rows as usize * stride);

        (decoder.output, 1, decoder.decoded_rows)
    } else {
        struct Luma8Decoder {
            output: Vec<u8>,
            columns: u32,
            white: u8,
            decoded_rows: u32,
        }

        impl Decoder for Luma8Decoder {
            fn push_row(&mut self, changes: &[u32]) {
                let start = self.decoded_rows as usize * self.columns as usize;
                let end = start + self.columns as usize;

                if self.output.len() < end {
                    self.output.resize(end, self.white);
                }

                let row = &mut self.output[start..end];

                for (start, end) in black_spans(changes, self.columns) {
                    fill_short(&mut row[start..end], !self.white);
                }

                self.decoded_rows += 1;
            }
        }

        let mut decoder = Luma8Decoder {
            output: vec![white; output_len],
            columns,
            white,
            decoded_rows: 0,
        };
        let result = hayro_ccitt::decode(data, &mut decoder, &settings);

        if result.is_err() && decoder.decoded_rows == 0 {
            return None;
        }

        if result.is_err() || rows == 0 {
            decoder
                .output
                .truncate(decoder.decoded_rows as usize * columns as usize);
        }

        (decoder.output, 8, decoder.decoded_rows)
    };

    Some(FilterResult {
        data: Cow::Owned(decoded),
        image_data: Some(ImageData {
            alpha: None,
            color_space: Some(ImageColorSpace::Gray),
            bits_per_component: bpc,
            width: columns,
            height: if rows == 0 { decoded_rows } else { rows },
        }),
    })
}

fn black_spans(changes: &[u32], columns: u32) -> impl Iterator<Item = (usize, usize)> + '_ {
    changes.chunks(2).map(move |span| {
        let end = span.get(1).copied().unwrap_or(columns);
        (span[0] as usize, end as usize)
    })
}

#[inline(always)]
fn fill_short<T: Copy>(slice: &mut [T], value: T) {
    let len = slice.len();

    // Overlapping fixed-size stores beat `memset` for spans up to 32.
    match len {
        0 => {}
        1..4 => {
            slice[0] = value;
            slice[len / 2] = value;
            slice[len - 1] = value;
        }
        4..8 => {
            slice[..4].copy_from_slice(&[value; 4]);
            slice[len - 4..].copy_from_slice(&[value; 4]);
        }
        8..16 => {
            slice[..8].copy_from_slice(&[value; 8]);
            slice[len - 8..].copy_from_slice(&[value; 8]);
        }
        16..=32 => {
            slice[..16].copy_from_slice(&[value; 16]);
            slice[len - 16..].copy_from_slice(&[value; 16]);
        }
        _ => slice.fill(value),
    }
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::object::FromBytes;
    use crate::object::dict::Dict;
    use crate::object::stream::ImageDecodeParams;

    #[test]
    fn issue1258() {
        let params = Dict::from_bytes(b"<< /K 0 /Columns 8 /Rows 1 >>").unwrap();

        let decoded = decode(&[0x35, 0x14], &params, &ImageDecodeParams::default()).unwrap();

        assert_eq!(decoded.data.as_ref(), &[0; 8]);
        assert_eq!(decoded.image_data.unwrap().height, 1);
    }

    #[test]
    fn unknown_rows() {
        let params = Dict::from_bytes(b"<< /K 0 /Columns 8 >>").unwrap();

        let decoded = decode(&[0x35, 0x14], &params, &ImageDecodeParams::default()).unwrap();
        assert_eq!(decoded.data.as_ref(), &[0; 8]);
        assert_eq!(decoded.image_data.unwrap().height, 1);

        let image_params = ImageDecodeParams {
            is_indexed: true,
            ..Default::default()
        };
        let decoded = decode(&[0x35, 0x14], &params, &image_params).unwrap();
        assert_eq!(decoded.data.as_ref(), &[0]);
        assert_eq!(decoded.image_data.unwrap().height, 1);
    }
}
