use crate::object::Dict;
use crate::object::dict::keys::{
    BLACK_IS_1, COLUMNS, ENCODED_BYTE_ALIGN, END_OF_BLOCK, END_OF_LINE, K, ROWS,
};
use hayro_ccitt::{DecodeSettings, Decoder, EncodingMode};

pub(crate) fn decode(data: &[u8], params: Dict<'_>) -> Option<Vec<u8>> {
    let k = params.get::<i32>(K).unwrap_or(0);

    let mut rows = params.get::<usize>(ROWS).unwrap_or(0) as u32;
    let end_of_block = params.get::<bool>(END_OF_BLOCK).unwrap_or(true);

    // hayro-ccitt's `end_of_block` defines whether the image MAY have an EOFB
    // block, but it will still use the `rows` attribute to check if decoding should
    // be stopped.  In PDF, it means whether it WILL have an EOFB, and the `rows`
    // attribute will be 0 then. Because of this, we set `rows` to max, so that
    // `hayro-ccitt` keeps decoding untilt he EOFB has been found, instead of
    // decoding 0 rows.
    if end_of_block {
        rows = u32::MAX;
    }

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
    let decode_res = hayro_ccitt::decode(data, &mut decoder, &settings);

    // We are lenient and return the image if at least one row as decoded
    // but the overall decoding process resulted in an error. However, if not
    // even a single scanline was decoded successfully, we return `None`.
    if decode_res.is_err() && decoder.output.is_empty() {
        return None;
    }

    Some(decoder.output)
}
