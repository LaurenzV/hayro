use crate::object::Dict;
use crate::object::dict::keys::{
    BLACK_IS_1, COLUMNS, ENCODED_BYTE_ALIGN, END_OF_BLOCK, END_OF_LINE, K, ROWS,
};
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;
use hayro_ccitt::{DecodeSettings, Decoder, DecoderContext, EncodingMode};

pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
) -> Option<FilterResult<'static>> {
    let k = params.get::<i32>(K).unwrap_or(0);

    let columns = params.get::<usize>(COLUMNS).unwrap_or(1728) as u32;
    // `Rows` should match the height of the image, but in case it doesn't,
    // the image height takes precedence (as in other viewers).
    let rows = if image_params.height > 0 {
        image_params.height
    } else {
        params.get::<u32>(ROWS).unwrap_or(0)
    };
    let output_len = (columns as usize).checked_mul(rows as usize)?;

    let mut settings = DecodeSettings::new(columns, rows);
    settings.end_of_block = params.get::<bool>(END_OF_BLOCK).unwrap_or(true);
    settings.end_of_line = params.get::<bool>(END_OF_LINE).unwrap_or(false);
    settings.rows_are_byte_aligned = params.get::<bool>(ENCODED_BYTE_ALIGN).unwrap_or(false);
    settings.encoding = if k < 0 {
        EncodingMode::Group4
    } else if k == 0 {
        EncodingMode::Group3_1D
    } else {
        EncodingMode::Group3_2D
    };
    settings.invert_black = params.get::<bool>(BLACK_IS_1).unwrap_or(false);

    // Whenever possible (if we don't have an indexed color space), we convert
    // the data as 8-bit instead of 1-bit, so that it can be easier converted
    // into an RGBA8 image.

    let (decoded, bpc, decoded_rows) = if image_params.is_indexed {
        struct BitPackDecoder {
            output: Vec<u8>,
            stride: usize,
            /// The bits of the last byte in a row that belong to the image.
            last_byte_mask: u8,
            row_start: usize,
            x: usize,
            decoded_rows: u32,
        }

        impl BitPackDecoder {
            fn row(&mut self) -> &mut [u8] {
                let end = self.row_start + self.stride;

                // If the number of rows is unknown, the output grows on demand.
                if self.output.len() < end {
                    self.output.resize(end, 0xFF);
                }

                &mut self.output[self.row_start..end]
            }
        }

        // The output is prefilled with white, so only black runs need to be
        // written.
        impl Decoder for BitPackDecoder {
            fn push_pixels(&mut self, white: bool, count: u32) {
                let start = self.x;
                self.x += count as usize;

                if white || count == 0 {
                    return;
                }

                let end = self.x;
                let row = self.row();
                let (first, last) = (start / 8, (end - 1) / 8);
                let first_mask = 0xFF_u8 >> (start % 8);
                let last_mask = 0xFF_u8 << (7 - (end - 1) % 8);

                if first == last {
                    row[first] &= !(first_mask & last_mask);
                } else {
                    row[first] &= !first_mask;
                    row[first + 1..last].fill(0x00);
                    row[last] &= !last_mask;
                }
            }

            fn next_line(&mut self) {
                let mask = self.last_byte_mask;

                if let Some(last) = self.row().last_mut() {
                    *last &= mask;
                }

                self.decoded_rows += 1;
                self.row_start += self.stride;
                self.x = 0;
            }
        }

        let stride = (columns as usize).div_ceil(8);
        let mut decoder = BitPackDecoder {
            output: vec![0xFF; stride.checked_mul(rows as usize)?],
            stride,
            last_byte_mask: 0xFF_u8 << ((8 - columns % 8) % 8),
            row_start: 0,
            x: 0,
            decoded_rows: 0,
        };
        let mut context = DecoderContext::new(settings);
        let result = hayro_ccitt::decode(data, &mut decoder, &mut context);

        // If we decoded at least one row, let's be lenient and return what we got.
        // See also 0001763.pdf.
        if result.is_err() && decoder.decoded_rows == 0 {
            return None;
        }

        decoder.output.truncate(decoder.row_start);

        (decoder.output, 1, decoder.decoded_rows)
    } else {
        struct Luma8Decoder {
            output: Vec<u8>,
            idx: usize,
            decoded_rows: u32,
        }

        impl Decoder for Luma8Decoder {
            fn push_pixels(&mut self, white: bool, count: u32) {
                let len = count as usize;

                if !white {
                    // If the number of rows is unknown, the output grows on demand.
                    if self.output.len() < self.idx + len {
                        self.output.resize(self.idx + len, 0xFF);
                    }

                    self.output[self.idx..self.idx + len].fill(0x00);
                }

                self.idx += len;
            }

            fn next_line(&mut self) {
                self.decoded_rows += 1;
            }
        }

        let mut decoder = Luma8Decoder {
            output: vec![0xFF; output_len],
            idx: 0,
            decoded_rows: 0,
        };
        let mut context = DecoderContext::new(settings);
        let result = hayro_ccitt::decode(data, &mut decoder, &mut context);

        if result.is_err() && decoder.decoded_rows == 0 {
            return None;
        }

        if result.is_err() || rows == 0 {
            decoder.output.resize(decoder.idx, 0xFF);
        }

        (decoder.output, 8, decoder.decoded_rows)
    };

    Some(FilterResult {
        data: Cow::Owned(decoded),
        image_data: Some(ImageData {
            alpha: None,
            color_space: Some(ImageColorSpace::Gray),
            bits_per_component: bpc,
            width: settings.columns,
            height: if rows == 0 { decoded_rows } else { rows },
        }),
    })
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
