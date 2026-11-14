use crate::bit_reader::BitReader;
use crate::context::{Color, DecoderContext, decode_1d_line, decode_2d_line};
use crate::{DecodeError, Decoder, EncodingMode, Result};

pub(crate) fn decode_1d(
    ctx: &mut DecoderContext<'_>,
    reader: &mut BitReader<'_>,
    decoder: &mut impl Decoder,
) -> Result<()> {
    // PDF producers often set `end_of_line` wrongly, so always look for an EOL.
    ctx.has_eols = reader.read_eol_if_available() > 0 || ctx.settings.end_of_line;

    loop {
        if ctx
            .settings
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.is_cancelled())
        {
            return Err(DecodeError::Cancelled);
        }

        let line_start = reader.clone();

        if let Err(e) = decode_1d_line(ctx, reader) {
            recover(ctx, reader, line_start, e)?;
        }

        ctx.next_line(decoder);

        if check_eob(ctx, reader)? {
            break;
        }
    }

    Ok(())
}

pub(crate) fn decode_2d(
    ctx: &mut DecoderContext<'_>,
    reader: &mut BitReader<'_>,
    decoder: &mut impl Decoder,
) -> Result<()> {
    // PDF producers often set `end_of_line` wrongly, so always look for an EOL.
    ctx.has_eols = reader.read_eol_if_available() > 0 || ctx.settings.end_of_line;

    loop {
        if ctx
            .settings
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.is_cancelled())
        {
            return Err(DecodeError::Cancelled);
        }

        let line_start = reader.clone();

        let result = reader.read_bit().and_then(|tag_bit| {
            if tag_bit == 1 {
                decode_1d_line(ctx, reader)
            } else {
                decode_2d_line(ctx, reader)
            }
        });

        if let Err(e) = result {
            recover(ctx, reader, line_start, e)?;
        }

        ctx.next_line(decoder);

        if check_eob(ctx, reader)? {
            break;
        }
    }

    Ok(())
}

#[cold]
fn recover<'a>(
    ctx: &mut DecoderContext<'_>,
    reader: &mut BitReader<'a>,
    line_start: BitReader<'a>,
    error: DecodeError,
) -> Result<()> {
    if !ctx.has_eols || ctx.settings.strict {
        return Err(error);
    }

    // The first EOL after the line start terminates the line, and `check_eob` skips to it.
    *reader = line_start;

    ctx.color = Color::White;
    ctx.push_pixels(ctx.line_width - ctx.pixels_decoded)
}

fn check_eob(ctx: &mut DecoderContext<'_>, reader: &mut BitReader<'_>) -> Result<bool> {
    // Without EOLs, alignment padding plus the next line's leading zeros could look like an EOL.
    if ctx.settings.rows_are_byte_aligned && !ctx.has_eols {
        reader.align();
    }

    let mut eol_count = read_eols(ctx, reader);
    let rows_done = ctx.decoded_rows == ctx.settings.rows;

    if eol_count == 0 && ctx.has_eols && !rows_done && !reader.at_end() {
        if ctx.settings.strict {
            return Err(DecodeError::InvalidCode);
        }

        if reader.skip_to_eol() {
            eol_count = read_eols(ctx, reader);
        } else {
            ctx.has_eols = false;
        }
    }

    // PDFBOX-2778 has 7 EOLs instead of 6.
    if ctx.settings.end_of_block && eol_count >= 6 {
        return Ok(true);
    }

    Ok(rows_done || reader.at_end())
}

// In 2D data an RTC is six EOL+1 sequences, so tag bits between EOLs are consumed too.
fn read_eols(ctx: &DecoderContext<'_>, reader: &mut BitReader<'_>) -> usize {
    let mut eol_count = reader.read_eol_if_available();

    if !matches!(ctx.settings.encoding, EncodingMode::Group3_2D) {
        return eol_count;
    }

    while eol_count > 0 {
        let mut next = reader.clone();

        if next.read_bit() != Ok(1) {
            break;
        }

        match next.read_eol_if_available() {
            0 => {
                // Unless this is the last tag bit of an RTC, it belongs to the next line.
                if eol_count >= 6 {
                    *reader = next;
                }

                break;
            }
            count => {
                *reader = next;
                eol_count += count;
            }
        }
    }

    eol_count
}
