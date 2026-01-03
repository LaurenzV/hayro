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
    }

    impl Decoder for ByteDecoder {
        fn push_byte(&mut self, byte: u8) {
            self.output.push(byte);
        }

        fn push_bytes(&mut self, byte: u8, count: usize) {
            self.output.extend(std::iter::repeat_n(byte, count));
        }

        fn next_line(&mut self) {
            self.decoded_rows += 1;
            // Nothing else to do here, as hayro-ccitt will already align to
            // byte-boundary after each row.
        }
    }

    let mut decoder = ByteDecoder {
        output: Vec::new(),
        decoded_rows: 0,
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
