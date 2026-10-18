use crate::bit_reader::BitReader;
use crate::context::{DecoderContext, decode_2d_line};
use crate::{Decoder, Result};

pub(crate) fn decode(
    ctx: &mut DecoderContext,
    reader: &mut BitReader<'_>,
    decoder: &mut impl Decoder,
) -> Result<()> {
    loop {
        // EOLs aren't part of Group 4, but PDF requires decoders to accept them
        // anyway. An EOFB consists of two consecutive EOLs. Data that follows
        // an EOFB (after padding to a byte boundary) can start like another
        // EOL, so no more than that is read.
        let eol_count = if ctx.settings.end_of_block {
            reader.read_eols(2)
        } else {
            reader.read_eol_if_available()
        };

        if ctx.settings.end_of_block && eol_count >= 2 {
            break;
        }

        if (ctx.settings.rows != 0 && ctx.decoded_rows == ctx.settings.rows) || reader.at_end() {
            break;
        }

        decode_2d_line(ctx, reader)?;
        ctx.next_line(decoder);

        if ctx.settings.rows_are_byte_aligned {
            reader.align();
        }
    }

    Ok(())
}
