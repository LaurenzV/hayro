use crate::bit_reader::BitWriter;
use crate::object::Dict;
use crate::object::Stream;
use crate::object::dict::keys::JBIG2_GLOBALS;

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

    let mut writer = BitWriter::new(&mut packed, 1)?;

    for row in image.data.chunks_exact(image.width as usize) {
        for &pixel in row {
            // We need to invert the color because JBIG2 uses black = 1 and
            // white = 0.
            writer.write(u32::from(!pixel))?;
        }

        // Images need to be padded to the byte boundary after each row.
        writer.align();
    }

    Some(packed)
}
