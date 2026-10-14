use crate::bit_reader::BitReader;
use crate::context::{Color, DecoderContext, decode_1d_line, decode_2d_line};
use crate::{DecodeError, Decoder, EncodingMode, Result};

/// Group 3 1D decoding (T.4 Section 4.1).
pub(crate) fn decode_1d(
    ctx: &mut DecoderContext,
    reader: &mut BitReader<'_>,
    decoder: &mut impl Decoder,
) -> Result<()> {
    // It seems like PDF producers are a bit sloppy with the `end_of_line` flag,
    // so we just always try to read one.
    ctx.has_eols = reader.read_eol_if_available() > 0 || ctx.settings.end_of_line;

    loop {
        let line_start = reader.clone();

        if let Err(e) = decode_1d_line(ctx, reader, decoder) {
            recover(ctx, reader, decoder, line_start, e)?;
        }

        ctx.next_line(decoder);

        if check_eob(ctx, reader)? {
            break;
        }
    }

    Ok(())
}

/// Group 3 2D decoding (T.4 Section 4.2).
pub(crate) fn decode_2d(
    ctx: &mut DecoderContext,
    reader: &mut BitReader<'_>,
    decoder: &mut impl Decoder,
) -> Result<()> {
    // It seems like PDF producers are a bit sloppy with the `end_of_line` flag,
    // so we just always try to read one.
    ctx.has_eols = reader.read_eol_if_available() > 0 || ctx.settings.end_of_line;

    loop {
        let line_start = reader.clone();

        let result = reader.read_bit().and_then(|tag_bit| {
            if tag_bit == 1 {
                decode_1d_line(ctx, reader, decoder)
            } else {
                decode_2d_line(ctx, reader, decoder)
            }
        });

        if let Err(e) = result {
            recover(ctx, reader, decoder, line_start, e)?;
        }

        ctx.next_line(decoder);

        if check_eob(ctx, reader)? {
            break;
        }
    }

    Ok(())
}

/// Recover from a damaged line by continuing at the next EOL.
#[cold]
fn recover<'a>(
    ctx: &mut DecoderContext,
    reader: &mut BitReader<'a>,
    decoder: &mut impl Decoder,
    line_start: BitReader<'a>,
    error: DecodeError,
) -> Result<()> {
    if !ctx.has_eols || ctx.settings.strict {
        return Err(error);
    }

    // Valid data never contains an EOL, so the first EOL after the start of
    // the line must be the one terminating it. Since the line won't be
    // followed by an EOL from its start, `check_eob` will skip to it.
    *reader = line_start;

    ctx.color = Color::White;
    ctx.push_pixels(decoder, ctx.line_width - ctx.pixels_decoded)
}

/// Check for end-of-block, including RTC (T.4 Section 4.1.4).
fn check_eob(ctx: &mut DecoderContext, reader: &mut BitReader<'_>) -> Result<bool> {
    // If the data contains EOLs, we treat the zeros used for byte alignment as
    // fill, so it doesn't matter whether the encoder aligned the start or the
    // end of the EOL. Without EOLs, we need to align first, as the padding
    // together with the leading zeros of the next line could otherwise be
    // mistaken for an EOL.
    if ctx.settings.rows_are_byte_aligned && !ctx.has_eols {
        reader.align();
    }

    let mut eol_count = read_eols(ctx, reader);
    let rows_done = ctx.decoded_rows == ctx.settings.rows;

    // If a line isn't followed by an EOL even though the data contains EOLs,
    // the line contained more data than expected, so we skip the rest.
    if eol_count == 0 && ctx.has_eols && !rows_done && !reader.at_end() {
        if ctx.settings.strict {
            return Err(DecodeError::InvalidCode);
        }

        if reader.skip_to_eol() {
            eol_count = read_eols(ctx, reader);
        } else {
            // There are no more EOLs, so don't search for them again.
            ctx.has_eols = false;
        }
    }

    // T.4 Section 4.1.4: "The end of a document transmission is indicated by
    // sending six consecutive EOLs."
    // PDFBOX-2778 has 7 EOL, although it should only be 6. Let's be lenient
    // and check with >=.
    if ctx.settings.end_of_block && eol_count >= 6 {
        return Ok(true);
    }

    Ok(rows_done || reader.at_end())
}

/// Read consecutive EOLs.
///
/// In 2D data, an RTC consists of six times EOL + 1 (T.4 Section 4.2.4), so
/// tag bits between EOLs are skipped as well, including the one after the last
/// EOL of an RTC.
fn read_eols(ctx: &DecoderContext, reader: &mut BitReader<'_>) -> usize {
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
                // Unless this is the last tag bit of an RTC, it belongs to the
                // next line.
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
