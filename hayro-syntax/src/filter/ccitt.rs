use crate::limits::Limits;
use crate::object::Dict;
use crate::object::dict::keys::{
    BLACK_IS_1, COLUMNS, ENCODED_BYTE_ALIGN, END_OF_BLOCK, END_OF_LINE, K, ROWS,
};
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;
use core::iter;
use hayro_ccitt::{DecodeSettings, Decoder, DecoderContext, EncodingMode};

pub(crate) fn decode(
    data: &[u8],
    params: &Dict<'_>,
    image_params: &ImageDecodeParams,
    limits: Limits,
) -> Option<FilterResult<'static>> {
    let k = params.get::<i32>(K).unwrap_or(0);

    let columns = params.get::<usize>(COLUMNS).unwrap_or(1728) as u32;
    let rows = params.get::<u32>(ROWS).unwrap_or(image_params.height);

    // `columns`/`rows` come from the decode parameters, independent of the
    // image dictionary's dimensions, and size the output buffer below — so
    // enforce the limit against them directly.
    if !limits.permits_image(columns, rows) {
        debug!("CCITT image {columns}x{rows} exceeds the configured limits");
        return None;
    }

    let output_len = (columns as usize).checked_mul(rows as usize)?;
    let end_of_block = params.get::<bool>(END_OF_BLOCK).unwrap_or(true);

    let settings = DecodeSettings {
        columns,
        rows,
        end_of_block,
        end_of_line: params.get::<bool>(END_OF_LINE).unwrap_or(false),
        rows_are_byte_aligned: params.get::<bool>(ENCODED_BYTE_ALIGN).unwrap_or(false),
        encoding: if k < 0 {
            EncodingMode::Group4
        } else if k == 0 {
            EncodingMode::Group3_1D
        } else {
            EncodingMode::Group3_2D { k: k as u32 }
        },
        invert_black: params.get::<bool>(BLACK_IS_1).unwrap_or(false),
    };

    // Whenever possible (if we don't have an indexed color space), we convert
    // the data as 8-bit instead of 1-bit, so that it can be easier converted
    // into an RGBA8 image.

    let (decoded, bpc) = if image_params.is_indexed {
        struct BitPackDecoder {
            output: Vec<u8>,
            decoded_rows: u32,
            buffer: u8,
            bit_count: u8,
        }

        impl BitPackDecoder {
            fn flush(&mut self) {
                if self.bit_count > 0 {
                    let padded = self.buffer << (8 - self.bit_count);
                    self.output.push(padded);
                    self.buffer = 0;
                    self.bit_count = 0;
                }
            }
        }

        impl Decoder for BitPackDecoder {
            fn push_pixels(&mut self, white: bool, mut count: u32) {
                if self.bit_count > 0 {
                    let head = count.min(u32::from(8 - self.bit_count)) as u8;
                    self.buffer <<= head;
                    if white {
                        self.buffer |= ((1_u16 << head) - 1) as u8;
                    }
                    self.bit_count += head;
                    count -= u32::from(head);

                    if self.bit_count == 8 {
                        self.output.push(self.buffer);
                        self.buffer = 0;
                        self.bit_count = 0;
                    }
                }

                let full_bytes = count / 8;
                if full_bytes > 0 {
                    let byte = if white { 0xFF } else { 0x00 };
                    self.output
                        .extend(iter::repeat_n(byte, full_bytes as usize));
                    count %= 8;
                }

                if count > 0 {
                    self.buffer = if white {
                        ((1_u16 << count) - 1) as u8
                    } else {
                        0
                    };
                    self.bit_count = count as u8;
                }
            }

            fn next_line(&mut self) {
                self.decoded_rows += 1;
                self.flush();
            }
        }

        let mut decoder = BitPackDecoder {
            output: Vec::new(),
            decoded_rows: 0,
            buffer: 0,
            bit_count: 0,
        };
        let mut context = DecoderContext::new(settings);
        let result = hayro_ccitt::decode(data, &mut decoder, &mut context);

        // If we decoded at least one row, let's be lenient and return what we got.
        // See also 0001763.pdf.
        if result.is_err() && decoder.decoded_rows == 0 {
            return None;
        }

        (decoder.output, 1)
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

        if result.is_err() {
            decoder.output.truncate(decoder.idx);
        }

        (decoder.output, 8)
    };

    Some(FilterResult {
        data: Cow::Owned(decoded),
        image_data: Some(ImageData {
            alpha: None,
            color_space: Some(ImageColorSpace::Gray),
            bits_per_component: bpc,
            width: settings.columns,
            height: rows,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::limits::{Limit, Limits};
    use crate::object::FromBytes;
    use crate::object::dict::Dict;
    use crate::object::stream::ImageDecodeParams;

    #[test]
    fn issue1258() {
        let params = Dict::from_bytes(b"<< /K 0 /Columns 8 /Rows 1 >>").unwrap();

        let decoded = decode(
            &[0x35, 0x14],
            &params,
            &ImageDecodeParams::default(),
            Limits::default(),
        )
        .unwrap();

        assert_eq!(decoded.data.as_ref(), &[0; 8]);
        assert_eq!(decoded.image_data.unwrap().height, 1);
    }

    #[test]
    fn rejects_oversized_dimensions() {
        // `columns * rows` sizes the output buffer independently of the image
        // dictionary, so an absurd /Columns must be refused under a limit.
        let params = Dict::from_bytes(b"<< /K 0 /Columns 100000 /Rows 100000 >>").unwrap();
        let limits = Limits {
            max_image_pixels: Limit::AtMost(1_000_000),
            ..Limits::default()
        };
        assert!(decode(&[0x00], &params, &ImageDecodeParams::default(), limits).is_none());
    }
}
