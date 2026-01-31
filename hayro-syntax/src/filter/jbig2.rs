use crate::bit_reader::BitWriter;
use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;
use alloc::vec;
use alloc::vec::Vec;

/// Decode JBIG2 data from a PDF stream.
///
/// The `params` dictionary may contain a `JBIG2Globals` entry pointing to
/// a stream with shared symbol dictionaries.
pub(crate) fn decode(data: &[u8], params: Dict<'_>) -> Option<Vec<u8>> {
    let globals = params
        .get::<Stream<'_>>(JBIG2_GLOBALS)
        .and_then(|g| g.decoded().ok());

    let image = hayro_jbig2::decode_embedded(data, globals.as_deref()).ok()?;

    let row_bytes = (image.width as usize).div_ceil(8);
    let mut packed = vec![0_u8; row_bytes * image.height as usize];

    struct BitWriterDecoder<'a> {
        writer: BitWriter<'a>,
    }

    // We need to invert the color because JBIG2 uses black = 1 and
    // white = 0, but PDF uses the opposite.
    impl hayro_jbig2::Decoder for BitWriterDecoder<'_> {
        fn push_pixel(&mut self, black: bool) {
            let _ = self.writer.write(u32::from(!black));
        }

        fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
            let byte_value = if black { 0x00 } else { 0xFF };
            let _ = self.writer.fill_bytes(byte_value, chunk_count as usize);
        }

        fn next_line(&mut self) {
            // Images need to be padded to the byte boundary after each row.
            self.writer.align();
        }
    }

    let writer = BitWriter::new(&mut packed, 1)?;
    let mut decoder = BitWriterDecoder { writer };
    image.decode(&mut decoder);

    Some(packed)
}
