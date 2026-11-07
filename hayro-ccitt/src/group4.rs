use crate::bit_reader::BitReader;
use crate::context::{Color, DecoderContext, decode_2d_line};
use crate::{DecodeError, Decoder, Result};

pub(crate) fn decode(
    ctx: &mut DecoderContext,
    reader: &mut BitReader<'_>,
    decoder: &mut impl Decoder,
) -> Result<()> {
    loop {
        if ctx
            .settings
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.is_cancelled())
        {
            return Err(DecodeError::Cancelled);
        }

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

        if let Err(e) = decode_2d_line(ctx, reader) {
            if !ctx.settings.strict
                && !matches!(e, DecodeError::Cancelled | DecodeError::OutOfMemory)
                && ctx.pixels_decoded > 0
            {
                ctx.color = Color::White;
                if ctx.push_pixels(ctx.line_width - ctx.pixels_decoded).is_ok() {
                    ctx.next_line(decoder);
                }
            }
            return Err(e);
        }
        ctx.next_line(decoder);

        ctx.align_row(reader);
    }

    Ok(())
}
