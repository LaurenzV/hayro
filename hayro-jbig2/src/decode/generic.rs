//! Generic region segment parsing and decoding (7.4.6, 6.2).

use super::{
    AdaptiveTemplatePixel, RegionBitmap, RegionSegmentInfo, Template, parse_region_segment_info,
};
use crate::ScratchBuffers;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::{Bitmap, WORD_BITS, WORD_BYTES, WORD_SHIFT, Word};
use crate::error::{ParseError, RegionError, Result, TemplateError, bail};
use crate::reader::Reader;

/// Generic region decoding procedure (6.2).
pub(crate) fn decode(
    header: &GenericRegionHeader<'_>,
    ctx: &mut ScratchBuffers,
) -> Result<RegionBitmap> {
    let mut bitmap = Bitmap::new_with(
        header.region_info.width,
        header.region_info.height,
        header.region_info.x_location,
        header.region_info.y_location,
        false,
    )?;

    decode_into(header, &mut bitmap, ctx)?;

    Ok(RegionBitmap {
        bitmap,
        combination_operator: header.region_info.combination_operator,
    })
}

pub(crate) fn decode_into(
    header: &GenericRegionHeader<'_>,
    bitmap: &mut Bitmap,
    ctx: &mut ScratchBuffers,
) -> Result<()> {
    let data = header.data;

    if header.mmr {
        // "6.2.6 Decoding using MMR coding"
        let _ = decode_bitmap_mmr(bitmap, data, header.strict)?;
    } else {
        let mut decoder = ArithmeticDecoder::new(data);
        ctx.contexts.clear();
        ctx.contexts.resize(
            1 << header.template.context_bits(),
            ArithmeticDecoderContext::default(),
        );

        // "6.2.5 Decoding using a template and arithmetic coding"
        match &header.adaptive_template_pixels {
            GenericTemplatePixels::Regular(at_pixels) => decode_bitmap_arithmetic_coding(
                bitmap,
                &mut decoder,
                &mut ctx.contexts,
                header.template,
                header.tpgdon,
                at_pixels,
            )?,
            GenericTemplatePixels::Extended(at_pixels) => decode_bitmap_extended_template(
                bitmap,
                &mut decoder,
                &mut ctx.contexts,
                header.tpgdon,
                at_pixels,
            ),
        }
    }

    Ok(())
}

/// Parsed generic region segment header (7.4.6.1).
#[derive(Debug, Clone)]
pub(crate) struct GenericRegionHeader<'a> {
    pub(crate) region_info: RegionSegmentInfo,
    pub(crate) mmr: bool,
    pub(crate) template: Template,
    pub(crate) tpgdon: bool,
    pub(crate) adaptive_template_pixels: GenericTemplatePixels,
    pub(crate) data: &'a [u8],
    pub(crate) strict: bool,
}

/// The adaptive template pixels of a generic region (7.4.6.3).
#[derive(Debug, Clone)]
pub(crate) enum GenericTemplatePixels {
    /// GBTEMPLATE 0 to 3.
    Regular([AdaptiveTemplatePixel; 4]),
    /// A1 to A12 of the extended template, if not all at their nominal locations.
    Extended([AdaptiveTemplatePixel; 12]),
}

/// Parse a generic region segment header (7.4.6.1).
pub(crate) fn parse<'a>(
    reader: &mut Reader<'a>,
    had_unknown_length: bool,
    strict: bool,
) -> Result<GenericRegionHeader<'a>> {
    let mut region_info = parse_region_segment_info(reader)?;
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let mmr = flags & 0x01 != 0;

    let template = Template::from_byte(flags >> 1);
    let tpgdon = flags & 0x08 != 0;
    // The extended template only exists for GBTEMPLATE 0 (Table 2).
    let ext_template = flags & 0x10 != 0 && !mmr && template == Template::Template0;
    let adaptive_template_pixels = if ext_template {
        let mut pixels = [AdaptiveTemplatePixel::default(); 12];
        for pixel in &mut pixels {
            *pixel = parse_adaptive_template_pixel(reader)?;
        }

        // At their nominal locations, the 12 AT pixels form the same contexts
        // as template 0 with its default AT pixels.
        let nominal = pixels
            .iter()
            .zip(EXTENDED_TEMPLATE_NOMINAL)
            .all(|(pixel, (x, y))| pixel.x == x && pixel.y == y);

        if nominal {
            GenericTemplatePixels::Regular(
                TEMPLATE0_DEFAULT.map(|(x, y)| AdaptiveTemplatePixel { x, y }),
            )
        } else {
            GenericTemplatePixels::Extended(pixels)
        }
    } else if mmr {
        GenericTemplatePixels::Regular([AdaptiveTemplatePixel::default(); 4])
    } else {
        GenericTemplatePixels::Regular(parse_adaptive_template_pixels(reader, template)?)
    };
    let mut data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    // "As a special case, as noted in 7.2.7, an immediate generic region segment
    // may have an unknown length. In this case, it also indicates the height of
    // the generic region (i.e. the number of rows that have been decoded in this
    // segment; it must be no greater than the region segment bitmap height value
    // in the segment's region segment information field." (7.4.6.4)
    if had_unknown_length {
        // Length has already been validated during segment parsing.
        let (head, tail) = data.split_at(data.len() - 4);
        let row_count = u32::from_be_bytes(tail.try_into().unwrap());

        if row_count > region_info.height {
            bail!(RegionError::InvalidDimension);
        }

        region_info.height = row_count;
        data = head;
    }

    Ok(GenericRegionHeader {
        region_info,
        mmr,
        template,
        tpgdon,
        adaptive_template_pixels,
        data,
        strict,
    })
}

/// Parse adaptive template pixel positions (7.4.6.3).
pub(crate) fn parse_adaptive_template_pixels(
    reader: &mut Reader<'_>,
    template: Template,
) -> Result<[AdaptiveTemplatePixel; 4]> {
    let num_pixels = template.adaptive_template_pixels() as usize;

    let mut pixels = [AdaptiveTemplatePixel::default(); 4];

    for pixel in pixels.iter_mut().take(num_pixels) {
        *pixel = parse_adaptive_template_pixel(reader)?;
    }

    Ok(pixels)
}

/// Parse the location of one adaptive template pixel (7.4.6.3).
fn parse_adaptive_template_pixel(reader: &mut Reader<'_>) -> Result<AdaptiveTemplatePixel> {
    let x = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;
    let y = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;

    // Validate AT pixel location (6.2.5.4, Figure 7).
    // AT pixels must reference already-decoded pixels:
    // - y must be <= 0 (current row or above)
    // - if y == 0, x must be < 0 (strictly to the left of current pixel)
    if y > 0 || (y == 0 && x >= 0) {
        bail!(TemplateError::InvalidAtPixel);
    }

    Ok(AdaptiveTemplatePixel { x, y })
}

/// The default AT pixels of template 0 (Figure 3(a)).
const TEMPLATE0_DEFAULT: [(i8, i8); 4] = [(3, -1), (-3, -1), (2, -2), (-2, -2)];

/// The nominal locations of A1 to A12 (Figure 3(b)).
const EXTENDED_TEMPLATE_NOMINAL: [(i8, i8); 12] = [
    (-2, 0),
    (0, -2),
    (-2, -1),
    (-1, -2),
    (1, -2),
    (2, -1),
    (-3, 0),
    (-4, 0),
    (2, -2),
    (3, -1),
    (-2, -2),
    (-3, -1),
];

/// Whether the adaptive template pixels correspond to the default ones.
/// See Table 5.
fn has_default_at_pixels(template: Template, at_pixels: &[AdaptiveTemplatePixel; 4]) -> bool {
    match template {
        Template::Template0 => {
            at_pixels[0].x == 3
                && at_pixels[0].y == -1
                && at_pixels[1].x == -3
                && at_pixels[1].y == -1
                && at_pixels[2].x == 2
                && at_pixels[2].y == -2
                && at_pixels[3].x == -2
                && at_pixels[3].y == -2
        }
        Template::Template1 => at_pixels[0].x == 3 && at_pixels[0].y == -1,
        Template::Template2 => at_pixels[0].x == 2 && at_pixels[0].y == -1,
        Template::Template3 => at_pixels[0].x == 2 && at_pixels[0].y == -1,
    }
}

/// Decode a bitmap using MMR coding (6.2.6).
pub(crate) fn decode_bitmap_mmr(bitmap: &mut Bitmap, data: &[u8], strict: bool) -> Result<usize> {
    // hayro-ccitt treats 0 rows as "decode until the end of the data".
    if bitmap.height == 0 {
        return Ok(0);
    }

    /// A decoder sink that writes decoded rows into a `Bitmap`.
    struct BitmapDecoder<'a> {
        bitmap: &'a mut Bitmap,
        y: u32,
    }

    impl hayro_ccitt::Decoder for BitmapDecoder<'_> {
        #[inline]
        fn push_row(&mut self, changes: &[u32]) {
            let stride = self.bitmap.stride as usize;
            let row_start = self.y as usize * stride;
            let row = &mut self.bitmap.data[row_start..row_start + stride];

            for span in changes.chunks(2) {
                let start = span[0];
                let end = span.get(1).copied().unwrap_or(self.bitmap.width);
                let first_word = (start / WORD_BITS) as usize;
                let last_word = ((end - 1) / WORD_BITS) as usize;
                let first_mask = Word::MAX >> (start % WORD_BITS);
                let end_bit = end % WORD_BITS;
                let last_mask = if end_bit == 0 {
                    Word::MAX
                } else {
                    Word::MAX << (WORD_BITS - end_bit)
                };

                if first_word == last_word {
                    row[first_word] |= first_mask & last_mask;
                } else {
                    row[first_word] |= first_mask;
                    fill_short(&mut row[first_word + 1..last_word], Word::MAX);
                    row[last_word] |= last_mask;
                }
            }

            self.y += 1;
        }
    }

    let width = bitmap.width;
    let height = bitmap.height;
    let mut decoder = BitmapDecoder { bitmap, y: 0 };

    let mut settings = hayro_ccitt::DecodeSettings::new(width, height);
    // "If the number of bytes contained in the encoded bitmap is known in
    // advance, then it is permissible for the data stream not to contain
    // an EOFB" (6.2.6). But it _can_ contain it, which is what this
    // flag indicates.
    settings.end_of_block = true;
    settings.encoding = hayro_ccitt::EncodingMode::Group4;
    settings.strict = strict;

    // "An invocation of the generic region decoding procedure with MMR equal to
    // 1 shall consume an integral number of bytes, beginning and ending on a
    // byte boundary. This may involve skipping over some bits in the last byte
    // read." (6.2.6)
    //
    // hayro-ccitt already aligns to the byte boundary before returning, so
    // nothing else to do here.
    let mut context = hayro_ccitt::DecoderContext::new(settings);
    let result = hayro_ccitt::decode(data, &mut decoder, &mut context);

    if strict && (result.is_err() || decoder.y < height) {
        bail!(RegionError::InvalidMmrData);
    }

    Ok(result.unwrap_or(data.len()))
}

#[inline(always)]
fn fill_short<T: Copy>(slice: &mut [T], value: T) {
    let len = slice.len();

    // Most spans are short, and for them a `memset` call costs more than the
    // stores themselves. Overlapping fixed-size stores cover every length up
    // to 32.
    match len {
        0 => {}
        1..4 => {
            slice[0] = value;
            slice[len / 2] = value;
            slice[len - 1] = value;
        }
        4..8 => {
            slice[..4].copy_from_slice(&[value; 4]);
            slice[len - 4..].copy_from_slice(&[value; 4]);
        }
        8..16 => {
            slice[..8].copy_from_slice(&[value; 8]);
            slice[len - 8..].copy_from_slice(&[value; 8]);
        }
        16..=32 => {
            slice[..16].copy_from_slice(&[value; 16]);
            slice[len - 16..].copy_from_slice(&[value; 16]);
        }
        _ => slice.fill(value),
    }
}

// I'm not sure why, but I was getting very weird codegen (with bad performance)
// when attempting to do this via generics. Hence why we use a macro for that.
macro_rules! decode_loop {
    ($bitmap:expr, $decoder:expr, $contexts:expr, $ctx_gatherer:expr,
     $tpgdon:expr, $sltp_context:expr, $gather:expr) => {{
        let bitmap: &mut Bitmap = $bitmap;
        let decoder: &mut ArithmeticDecoder<'_> = $decoder;
        let contexts: &mut [ArithmeticDecoderContext] = $contexts;
        let ctx_gatherer: &mut ContextGatherer<'_> = $ctx_gatherer;
        let width = bitmap.width;
        let height = bitmap.height;

        // "1) Set: LTP = 0" (6.2.5.7)
        let mut ltp = false;

        // "3) Decode each row as follows:" (6.2.5.7)
        for y in 0..height {
            // "b) If TPGDON is 1, then decode a bit using the arithmetic entropy
            // coder" (6.2.5.7)
            if $tpgdon {
                let sltp = decoder.read_bit(&mut contexts[$sltp_context as usize]);
                // "Let SLTP be the value of this bit. Set: LTP = LTP XOR SLTP" (6.2.5.7)
                ltp = ltp != (sltp != 0);
            }

            // "c) If LTP = 1 then set every pixel of the current row of GBREG equal
            // to the corresponding pixel of the row immediately above." (6.2.5.7)
            if ltp {
                // If y == 0, pixels remain the same.
                if y > 0 {
                    let stride = bitmap.stride as usize;
                    let src = (y as usize - 1) * stride;
                    bitmap
                        .data
                        .copy_within(src..src + stride, y as usize * stride);
                }
            } else {
                // "d) If LTP = 0 then, from left to right, decode each pixel of the
                // current row of GBREG." (6.2.5.7)
                ctx_gatherer.start_row(bitmap, y);

                for x in 0..width {
                    ctx_gatherer.maybe_reload_buffers(bitmap, x);
                    let context_bits = ($gather)(ctx_gatherer, bitmap, x) as usize;
                    let pixel = decoder.read_bit(&mut contexts[context_bits]);
                    let value = pixel as u8;

                    if value != 0 {
                        bitmap.set_pixel(x, y, value);
                        ctx_gatherer.update_current_row(x, value);
                    }
                }
            }
        }
    }};
}

impl Bitmap {
    #[inline(always)]
    fn get_row_byte(&self, row: u32, byte_idx: usize) -> u32 {
        let word_idx = byte_idx / WORD_BYTES as usize;
        let shift = (WORD_BYTES as usize - 1 - byte_idx % WORD_BYTES as usize) * u8::BITS as usize;
        ((self.data[row as usize * self.stride as usize + word_idx] >> shift) & 0xff) as u8 as u32
    }

    #[inline(always)]
    fn set_row_byte(&mut self, row: u32, byte_idx: usize, value: u8) {
        let word_idx = byte_idx / WORD_BYTES as usize;
        let shift = (WORD_BYTES as usize - 1 - byte_idx % WORD_BYTES as usize) * u8::BITS as usize;
        let idx = row as usize * self.stride as usize + word_idx;
        let mask = Word::from(0xff_u8) << shift;
        self.data[idx] = (self.data[idx] & !mask) | ((value as Word) << shift);
    }
}

macro_rules! decode_default_template_fast_loop {
    (
        $bitmap:expr,
        $decoder:expr,
        $contexts:expr,
        $tpgdon:expr,
        $sltp_context:expr,
        $params:expr
    ) => {{
        const BYTE_BITS: u32 = u8::BITS;
        let stride = $bitmap.stride as usize;
        let mut ltp = false;

        let full_bytes = (($bitmap.width - 1) / BYTE_BITS) as usize;
        let last_byte_bits = (($bitmap.width - 1) & (BYTE_BITS - 1)) + 1;

        for y in 0..$bitmap.height {
            if $tpgdon {
                let sltp = $decoder.read_bit(&mut $contexts[$sltp_context as usize]);
                ltp = ltp != (sltp != 0);
            }

            if ltp {
                if y > 0 {
                    let src = (y as usize - 1) * stride;
                    $bitmap
                        .data
                        .copy_within(src..src + stride, y as usize * stride);
                }

                continue;
            }

            let has_prev2 = y >= 2;
            let has_prev1 = y >= 1;
            let mut val_prev2 = if has_prev2 {
                $bitmap.get_row_byte(y - 2, 0) << ($params).prev2_initial_shift
            } else {
                0
            };
            let mut val_prev1 = if has_prev1 {
                $bitmap.get_row_byte(y - 1, 0)
            } else {
                0
            };
            let mut context = (val_prev2 & ($params).prev2_initial_mask)
                | ((val_prev1 >> ($params).prev1_shift) & ($params).prev1_initial_mask);

            for byte_idx in 0..full_bytes {
                val_prev2 = if has_prev2 {
                    (val_prev2 << BYTE_BITS)
                        | ($bitmap.get_row_byte(y - 2, byte_idx + 1)
                            << ($params).prev2_initial_shift)
                } else {
                    0
                };
                val_prev1 = if has_prev1 {
                    (val_prev1 << BYTE_BITS) | $bitmap.get_row_byte(y - 1, byte_idx + 1)
                } else {
                    0
                };

                // If the context and the pixels the rows above bring in next are all
                // white (or all black), the context stays the same for all eight
                // pixels as long as they decode to that same color. If that color is
                // the context's MPS and the decoder can decode eight MPS without
                // renormalizing, all eight pixels have that color.
                let prev2_mask = ($params).prev2_next_mask * 0xff;
                let prev1_mask = ($params).prev1_next_mask * 0xff;
                let incoming_prev2 = val_prev2 & prev2_mask;
                let incoming_prev1 = (val_prev1 >> ($params).prev1_shift) & prev1_mask;
                let uniform_color = if context == 0 && incoming_prev2 == 0 && incoming_prev1 == 0 {
                    Some(0)
                } else if context == ($params).full_context
                    && incoming_prev2 == prev2_mask
                    && incoming_prev1 == prev1_mask
                {
                    Some(1)
                } else {
                    None
                };

                if let Some(color) = uniform_color {
                    let state = $contexts[context as usize];

                    if state.mps() == color && $decoder.decode_mps_run(state, BYTE_BITS) {
                        $bitmap.set_row_byte(y, byte_idx, if color == 1 { 0xff } else { 0 });
                        continue;
                    }
                }

                let mut byte = 0;

                for bit in (0..BYTE_BITS).rev() {
                    let pixel = $decoder.read_bit(&mut $contexts[context as usize]);
                    byte |= (pixel as u8) << bit;
                    context = ((context & ($params).shift_mask) << 1)
                        | pixel
                        | ((val_prev2 >> bit) & ($params).prev2_next_mask)
                        | ((val_prev1 >> (bit + ($params).prev1_shift))
                            & ($params).prev1_next_mask);
                }

                $bitmap.set_row_byte(y, byte_idx, byte);
            }

            val_prev2 <<= BYTE_BITS;
            val_prev1 <<= BYTE_BITS;
            let mut byte = 0;
            for k in 0..last_byte_bits {
                let pixel = $decoder.read_bit(&mut $contexts[context as usize]);
                byte |= (pixel as u8) << (u8::BITS - 1 - k);
                context = ((context & ($params).shift_mask) << 1)
                    | pixel
                    | ((val_prev2 >> (u8::BITS - 1 - k)) & ($params).prev2_next_mask)
                    | ((val_prev1 >> (u8::BITS - 1 + ($params).prev1_shift - k))
                        & ($params).prev1_next_mask);
            }
            $bitmap.set_row_byte(y, full_bytes, byte);
        }
    }};
}

/// Decode a bitmap using arithmetic coding (6.2.5).
pub(crate) fn decode_bitmap_arithmetic_coding(
    bitmap: &mut Bitmap,
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [ArithmeticDecoderContext],
    template: Template,
    tpgdon: bool,
    adaptive_template_pixels: &[AdaptiveTemplatePixel; 4],
) -> Result<()> {
    if bitmap.width == 0 || bitmap.height == 0 {
        return Ok(());
    }

    let mut ctx_gatherer = ContextGatherer::new(template, adaptive_template_pixels);
    let sltp_context = sltp_context(template);

    if ctx_gatherer.use_default_at {
        match template {
            Template::Template0 => decode_default_template_fast_loop!(
                bitmap,
                decoder,
                contexts,
                tpgdon,
                sltp_context,
                DEFAULT_TEMPLATE0_FAST_PARAMS
            ),
            Template::Template1 => decode_default_template_fast_loop!(
                bitmap,
                decoder,
                contexts,
                tpgdon,
                sltp_context,
                DEFAULT_TEMPLATE1_FAST_PARAMS
            ),
            Template::Template2 => decode_default_template_fast_loop!(
                bitmap,
                decoder,
                contexts,
                tpgdon,
                sltp_context,
                DEFAULT_TEMPLATE2_FAST_PARAMS
            ),
            Template::Template3 => decode_default_template_fast_loop!(
                bitmap,
                decoder,
                contexts,
                tpgdon,
                sltp_context,
                DEFAULT_TEMPLATE3_FAST_PARAMS
            ),
        }
    } else {
        match template {
            Template::Template0 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template0_custom
            ),
            Template::Template1 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template1_custom
            ),
            Template::Template2 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template2_custom
            ),
            Template::Template3 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template3_custom
            ),
        }
    }

    Ok(())
}

/// The context used to decode SLTP (Figures 8 to 11).
fn sltp_context(template: Template) -> u16 {
    match template {
        Template::Template0 => 0b1001101100100101,
        Template::Template1 => 0b0011110010101,
        Template::Template2 => 0b0011100101,
        Template::Template3 => 0b0110010101,
    }
}

/// The context bits of A1 to A12: the bits template 0 uses for their nominal
/// locations, so that the SLTP context stays the same.
const EXTENDED_TEMPLATE_BITS: [u32; 12] = [1, 13, 9, 14, 12, 5, 2, 3, 11, 4, 15, 10];

/// Decode a bitmap with the extended template (6.2.5).
fn decode_bitmap_extended_template(
    bitmap: &mut Bitmap,
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [ArithmeticDecoderContext],
    tpgdon: bool,
    at_pixels: &[AdaptiveTemplatePixel; 12],
) {
    let default_at_pixels = TEMPLATE0_DEFAULT.map(|(x, y)| AdaptiveTemplatePixel { x, y });
    let mut ctx_gatherer = ContextGatherer::new(Template::Template0, &default_at_pixels);

    let gather = |gatherer: &mut ContextGatherer<'_>, bitmap: &Bitmap, x: u32| {
        let y = gatherer.cur_y;
        let pixel = |dx: i32, dy: i32, bit: u32| {
            let value = bitmap.get_pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32);
            (value as u16) << bit
        };

        let mut context = pixel(-1, 0, 0) | pixel(1, -1, 6) | pixel(0, -1, 7) | pixel(-1, -1, 8);
        for (at, bit) in at_pixels.iter().zip(EXTENDED_TEMPLATE_BITS) {
            context |= pixel(at.x as i32, at.y as i32, bit);
        }

        context
    };

    decode_loop!(
        bitmap,
        decoder,
        contexts,
        &mut ctx_gatherer,
        tpgdon,
        sltp_context(Template::Template0),
        gather
    );
}

pub(crate) struct ContextGatherer<'a> {
    template: Template,
    at_pixels: &'a [AdaptiveTemplatePixel; 4],
    use_default_at: bool,
    /// Used in `maybe_reload_buffers` to determine how far to the right
    /// we might access pixels.
    max_right: u32,
    /// Current position.
    cur_y: u32,
    cur_x: u32,
    /// Pre-fetched pixel buffers for rows y-2, y-1, y.
    buf_m2: Word,
    buf_m1: Word,
    buf_cur: Word,
    /// The current context for all 3 rows.
    ctx: u16,
}

impl<'a> ContextGatherer<'a> {
    pub(crate) fn new(template: Template, at_pixels: &'a [AdaptiveTemplatePixel; 4]) -> Self {
        let use_default_at = has_default_at_pixels(template, at_pixels);
        let max_right = match template {
            Template::Template0 | Template::Template1 => {
                if use_default_at {
                    3
                } else {
                    2
                }
            }
            Template::Template2 | Template::Template3 => {
                if use_default_at {
                    2
                } else {
                    1
                }
            }
        };
        Self {
            template,
            at_pixels,
            use_default_at,
            max_right,
            cur_y: 0,
            buf_m2: 0,
            buf_m1: 0,
            buf_cur: 0,
            cur_x: 0,
            ctx: 0,
        }
    }

    const SHIFT_MASK_T0: u16 = 0b0110_0011_1100_1110;
    const SHIFT_MASK_T1: u16 = 0b0001_1101_1110_0110;
    const SHIFT_MASK_T2: u16 = 0b0000_0011_0111_0010;
    const SHIFT_MASK_T3: u16 = 0b0000_0011_1100_1110;

    const SHIFT_MASK_T0_DEFAULT: u16 = 0b1111_0111_1110_1110;
    const SHIFT_MASK_T1_DEFAULT: u16 = 0b0001_1101_1111_0110;
    const SHIFT_MASK_T2_DEFAULT: u16 = 0b0000_0011_0111_1010;
    const SHIFT_MASK_T3_DEFAULT: u16 = 0b0000_0011_1110_1110;

    pub(crate) fn start_row(&mut self, bitmap: &Bitmap, y: u32) {
        self.cur_y = y;
        self.cur_x = 0;

        self.buf_m2 = if y >= 2 {
            Self::load_word(bitmap, y - 2, 0)
        } else {
            0
        };
        self.buf_m1 = if y >= 1 {
            Self::load_word(bitmap, y - 1, 0)
        } else {
            0
        };
        self.buf_cur = 0;

        // Start initializing the contexts. Note that this won't load all initial
        // pixels yet, those will only be loaded after our first call to `gather`.
        // See 6.2.5.3 for the pixel positions.
        self.ctx = if self.use_default_at {
            self.init_context_default()
        } else {
            self.init_context_custom()
        };
    }

    #[inline]
    fn init_context_custom(&self) -> u16 {
        match self.template {
            Template::Template0 => {
                let m2 = Self::get_buf_pixel(self.buf_m2, 0);
                let m1 = (Self::get_buf_pixel(self.buf_m1, 0) << 1)
                    | Self::get_buf_pixel(self.buf_m1, 1);
                (m2 << 12) | (m1 << 5)
            }
            Template::Template1 => {
                let m2 = (Self::get_buf_pixel(self.buf_m2, 0) << 1)
                    | Self::get_buf_pixel(self.buf_m2, 1);
                let m1 = (Self::get_buf_pixel(self.buf_m1, 0) << 1)
                    | Self::get_buf_pixel(self.buf_m1, 1);
                (m2 << 9) | (m1 << 4)
            }
            Template::Template2 => {
                let m2 = Self::get_buf_pixel(self.buf_m2, 0);
                let m1 = Self::get_buf_pixel(self.buf_m1, 0);
                (m2 << 7) | (m1 << 3)
            }
            Template::Template3 => {
                let m1 = Self::get_buf_pixel(self.buf_m1, 0);
                m1 << 5
            }
        }
    }

    #[inline]
    fn init_context_default(&self) -> u16 {
        match self.template {
            Template::Template0 => {
                (Self::get_buf_pixel(self.buf_m2, 0) << 12)
                    | (Self::get_buf_pixel(self.buf_m2, 1) << 11)
                    | (Self::get_buf_pixel(self.buf_m1, 0) << 6)
                    | (Self::get_buf_pixel(self.buf_m1, 1) << 5)
                    | (Self::get_buf_pixel(self.buf_m1, 2) << 4)
            }
            Template::Template1 => {
                (Self::get_buf_pixel(self.buf_m2, 0) << 10)
                    | (Self::get_buf_pixel(self.buf_m2, 1) << 9)
                    | (Self::get_buf_pixel(self.buf_m1, 0) << 5)
                    | (Self::get_buf_pixel(self.buf_m1, 1) << 4)
                    | (Self::get_buf_pixel(self.buf_m1, 2) << 3)
            }
            Template::Template2 => {
                (Self::get_buf_pixel(self.buf_m2, 0) << 7)
                    | (Self::get_buf_pixel(self.buf_m1, 0) << 3)
                    | (Self::get_buf_pixel(self.buf_m1, 1) << 2)
            }
            Template::Template3 => {
                (Self::get_buf_pixel(self.buf_m1, 0) << 5)
                    | (Self::get_buf_pixel(self.buf_m1, 1) << 4)
            }
        }
    }

    #[inline(always)]
    pub(crate) fn load_word(bitmap: &Bitmap, row_y: u32, start_x: u32) -> Word {
        let word_idx = start_x / WORD_BITS;

        if start_x.is_multiple_of(WORD_BITS) {
            bitmap.get_word(row_y, word_idx)
        } else {
            let bit_offset = start_x % WORD_BITS;
            let word1 = bitmap.get_word(row_y, word_idx);
            let word2 = bitmap.get_word(row_y, word_idx + 1);
            (word1 << bit_offset) | (word2 >> (WORD_BITS - bit_offset))
        }
    }

    #[inline]
    pub(crate) fn get_buf_pixel(buf: Word, pos: u32) -> u16 {
        if pos < WORD_BITS {
            ((buf >> (WORD_SHIFT - pos)) & 1) as u16
        } else {
            0
        }
    }

    #[inline(always)]
    pub(crate) fn maybe_reload_buffers(&mut self, bitmap: &Bitmap, x: u32) {
        if x + self.max_right >= self.cur_x + WORD_BITS {
            let new_start = x.saturating_sub(4);
            self.cur_x = new_start;
            self.buf_m2 = if self.cur_y >= 2 {
                Self::load_word(bitmap, self.cur_y - 2, new_start)
            } else {
                0
            };
            self.buf_m1 = if self.cur_y >= 1 {
                Self::load_word(bitmap, self.cur_y - 1, new_start)
            } else {
                0
            };
            self.buf_cur = Self::load_word(bitmap, self.cur_y, new_start);
        }
    }

    #[inline(always)]
    fn gather_template0_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 12)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 5)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            // Note that the cast from i32 to u32 is deliberate here: Negative positions
            // should be resolved to 0, by casting negative i32 to u32 we end up with
            // a very large positive number which is guaranteed to be OOB. The same
            // applies to all other occurrences of this pattern (also in `generic_refinement.rs`).
            | ((bitmap.get_pixel((xi + self.at_pixels[3].x as i32) as u32, (yi + self.at_pixels[3].y as i32) as u32) as u16) << 15)
            | ((bitmap.get_pixel((xi + self.at_pixels[2].x as i32) as u32, (yi + self.at_pixels[2].y as i32) as u32) as u16) << 11)
            | ((bitmap.get_pixel((xi + self.at_pixels[1].x as i32) as u32, (yi + self.at_pixels[1].y as i32) as u32) as u16) << 10)
            | ((bitmap.get_pixel((xi + self.at_pixels[0].x as i32) as u32, (yi + self.at_pixels[0].y as i32) as u32) as u16) << 4);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T0) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template1_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 9)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | ((bitmap.get_pixel(
                (xi + self.at_pixels[0].x as i32) as u32,
                (yi + self.at_pixels[0].y as i32) as u32,
            ) as u16)
                << 3);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T1) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template2_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 7)
            | (Self::get_buf_pixel(self.buf_m1, bx + 1) << 3)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | ((bitmap.get_pixel(
                (xi + self.at_pixels[0].x as i32) as u32,
                (yi + self.at_pixels[0].y as i32) as u32,
            ) as u16)
                << 2);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T2) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template3_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m1, bx + 1) << 5)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | ((bitmap.get_pixel(
                (xi + self.at_pixels[0].x as i32) as u32,
                (yi + self.at_pixels[0].y as i32) as u32,
            ) as u16)
                << 4);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T3) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    pub(crate) fn gather_template0_default(&mut self, _bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 11)
            | (Self::get_buf_pixel(self.buf_m1, bx + 3) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T0_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    pub(crate) fn gather_template1_default(&mut self, _bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 9)
            | (Self::get_buf_pixel(self.buf_m1, bx + 3) << 3)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T1_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    pub(crate) fn gather_template2_default(&mut self, _bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 7)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 2)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T2_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    pub(crate) fn gather_template3_default(&mut self, _bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m1, bx + 2) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T3_DEFAULT) | new_pixels;
        self.ctx
    }

    /// Note: The caller must ensure that `gather` has been called for this `x`
    /// first.
    #[inline(always)]
    pub(crate) fn update_current_row(&mut self, x: u32, value: u8) {
        debug_assert!(x >= self.cur_x && x < self.cur_x + WORD_BITS);

        let bit_pos = WORD_SHIFT - (x - self.cur_x);
        #[allow(trivial_numeric_casts)]
        let mask = (1 as Word) << bit_pos;
        self.buf_cur = (self.buf_cur & !mask) | ((value as Word) << bit_pos);
    }
}

#[derive(Clone, Copy)]
struct DefaultTemplateFastParams {
    prev2_initial_shift: u32,
    prev2_initial_mask: u32,
    prev1_shift: u32,
    prev1_initial_mask: u32,
    shift_mask: u32,
    prev2_next_mask: u32,
    prev1_next_mask: u32,
    full_context: u32,
}

const DEFAULT_TEMPLATE0_FAST_PARAMS: DefaultTemplateFastParams = DefaultTemplateFastParams {
    prev2_initial_shift: 6,
    prev2_initial_mask: 0xf800,
    prev1_shift: 0,
    prev1_initial_mask: 0x07f0,
    shift_mask: 0x7bf7,
    prev2_next_mask: 0x0800,
    prev1_next_mask: 0x0010,
    full_context: 0xffff,
};

const DEFAULT_TEMPLATE1_FAST_PARAMS: DefaultTemplateFastParams = DefaultTemplateFastParams {
    prev2_initial_shift: 4,
    prev2_initial_mask: 0x1e00,
    prev1_shift: 1,
    prev1_initial_mask: 0x01f8,
    shift_mask: 0x0efb,
    prev2_next_mask: 0x0200,
    prev1_next_mask: 0x0008,
    full_context: 0x1fff,
};

const DEFAULT_TEMPLATE2_FAST_PARAMS: DefaultTemplateFastParams = DefaultTemplateFastParams {
    prev2_initial_shift: 1,
    prev2_initial_mask: 0x0380,
    prev1_shift: 3,
    prev1_initial_mask: 0x007c,
    shift_mask: 0x01bd,
    prev2_next_mask: 0x0080,
    prev1_next_mask: 0x0004,
    full_context: 0x03ff,
};

const DEFAULT_TEMPLATE3_FAST_PARAMS: DefaultTemplateFastParams = DefaultTemplateFastParams {
    prev2_initial_shift: 0,
    prev2_initial_mask: 0x0000,
    prev1_shift: 1,
    prev1_initial_mask: 0x03f0,
    shift_mask: 0x01f7,
    prev2_next_mask: 0x0000,
    prev1_next_mask: 0x0010,
    full_context: 0x03ff,
};
