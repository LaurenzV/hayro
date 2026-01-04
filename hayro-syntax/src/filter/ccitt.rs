use crate::object::Dict;
use crate::object::dict::keys::{
    BLACK_IS_1, COLUMNS, ENCODED_BYTE_ALIGN, END_OF_BLOCK, END_OF_LINE, K, ROWS,
};
use crate::object::stream::{FilterResult, ImageColorSpace, ImageData, ImageDecodeParams};
use hayro_ccitt::{DecodeSettings, Decoder, EncodingMode};

pub(crate) fn decode(
    data: &[u8],
    params: Dict<'_>,
    image_params: &ImageDecodeParams,
) -> Option<FilterResult> {
    let k = params.get::<i32>(K).unwrap_or(0);

    let rows = params.get::<u32>(ROWS).unwrap_or(image_params.height);
    let end_of_block = params.get::<bool>(END_OF_BLOCK).unwrap_or(true);

    let settings = DecodeSettings {
        columns: params.get::<usize>(COLUMNS).unwrap_or(1728) as u32,
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

    struct ByteDecoder {
        output: Vec<u8>,
        decoded_rows: u32,
        buffer: u8,
        bit_count: u8,
    }

    impl ByteDecoder {
        fn push_bit(&mut self, white: bool) {
            let bit = if white { 1 } else { 0 };
            self.buffer = (self.buffer << 1) | bit;
            self.bit_count += 1;

            if self.bit_count == 8 {
                self.output.push(self.buffer);
                self.buffer = 0;
                self.bit_count = 0;
            }
        }

        fn flush(&mut self) {
            if self.bit_count > 0 {
                let padded = self.buffer << (8 - self.bit_count);
                self.output.push(padded);
                self.buffer = 0;
                self.bit_count = 0;
            }
        }
    }

    impl Decoder for ByteDecoder {
        fn push_pixel(&mut self, white: bool) {
            self.push_bit(white);
        }

        fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
            let byte = if white { 0xFF } else { 0x00 };
            self.output
                .extend(std::iter::repeat_n(byte, chunk_count as usize));
        }

        fn next_line(&mut self) {
            self.decoded_rows += 1;
            // Flush any remaining bits and align to byte boundary.
            self.flush();
        }
    }

    let mut decoder = ByteDecoder {
        output: Vec::new(),
        decoded_rows: 0,
        buffer: 0,
        bit_count: 0,
    };
    let result = hayro_ccitt::decode(data, &mut decoder, &settings);

    // If we decoded at least one row, let's be lenient and return what we got.
    // See also 0001763.pdf.
    if result.is_err() && decoder.decoded_rows == 0 {
        return None;
    }

    Some(FilterResult {
        data: decoder.output,
        image_data: Some(ImageData {
            alpha: None,
            color_space: Some(ImageColorSpace::Gray),
            bits_per_component: 1,
            width: settings.columns,
            height: image_params.height,
        }),
    })
}
