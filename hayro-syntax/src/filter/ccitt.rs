use crate::object::Dict;
use crate::object::dict::keys::{
    BLACK_IS_1, COLUMNS, ENCODED_BYTE_ALIGN, END_OF_BLOCK, END_OF_LINE, K, ROWS,
};
use hayro_ccitt::{DecodeSettings, Decoder, EncodingMode};

pub(crate) fn decode(data: &[u8], params: Dict<'_>) -> Option<Vec<u8>> {
    let k = params.get::<i32>(K).unwrap_or(0);

    let settings = DecodeSettings {
        strict: false,
        columns: params.get::<usize>(COLUMNS).unwrap_or(1728) as u32,
        rows: params.get::<usize>(ROWS).unwrap_or(0) as u32,
        end_of_block: params.get::<bool>(END_OF_BLOCK).unwrap_or(true),
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
    }

    impl Decoder for ByteDecoder {
        fn push_byte(&mut self, byte: u8) {
            self.output.push(byte);
        }

        fn push_bytes(&mut self, byte: u8, count: usize) {
            self.output.extend(std::iter::repeat_n(byte, count));
        }

        fn next_line(&mut self) {
            // Nothing to do here, as hayro-ccitt will already align to
            // byte-boundary after each row.
        }
    }

    let mut decoder = ByteDecoder { output: Vec::new() };
    hayro_ccitt::decode(data, &mut decoder, &settings);

    Some(decoder.output)
}
