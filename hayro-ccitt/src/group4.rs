use crate::bit_reader::BitReader;
use crate::context::{Color, DecoderContext, decode_2d_line};
use crate::{DecodeError, Decoder, Result};

pub(crate) fn decode(
    ctx: &mut DecoderContext<'_>,
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

        // PDF allows EOLs; read at most an EOFB's two, as data after it can look like an EOL.
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
            if ctx.settings.strict {
                return Err(e);
            }

            if ctx.pixels_decoded > 0 {
                ctx.color = Color::White;
                ctx.push_pixels(ctx.line_width - ctx.pixels_decoded)?;
                ctx.next_line(decoder);
            }

            if e == DecodeError::UnexpectedEof {
                break;
            }

            return Err(e);
        }

        ctx.next_line(decoder);

        if ctx.settings.rows_are_byte_aligned {
            reader.align();
        }
    }

    Ok(())
}
