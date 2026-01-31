//! Generic region segment parsing and decoding (7.4.6, 6.2).

use alloc::vec;
use alloc::vec::Vec;

use super::{
    AdaptiveTemplatePixel, RegionBitmap, RegionSegmentInfo, Template, parse_region_segment_info,
};
use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::Bitmap;
use crate::error::{ParseError, RegionError, Result, TemplateError, bail};
use crate::reader::Reader;

/// Generic region decoding procedure (6.2).
pub(crate) fn decode(header: &GenericRegionHeader<'_>) -> Result<RegionBitmap> {
    let data = header.data;
    let mut bitmap = Bitmap::new_with(
        header.region_info.width,
        header.region_info.height,
        header.region_info.x_location,
        header.region_info.y_location,
        false,
    );

    if header.mmr {
        // "6.2.6 Decoding using MMR coding"
        let _ = decode_bitmap_mmr(&mut bitmap, data)?;
    } else {
        let mut decoder = ArithmeticDecoder::new(data);
        let mut contexts = vec![Context::default(); 1 << header.template.context_bits()];

        // "6.2.5 Decoding using a template and arithmetic coding"
        decode_bitmap_arithmetic_coding(
            &mut bitmap,
            &mut decoder,
            &mut contexts,
            header.template,
            header.tpgdon,
            &header.adaptive_template_pixels,
        )?;
    }

    Ok(RegionBitmap {
        bitmap,
        combination_operator: header.region_info.combination_operator,
    })
}

/// Parsed generic region segment header (7.4.6.1).
#[derive(Debug, Clone)]
pub(crate) struct GenericRegionHeader<'a> {
    pub(crate) region_info: RegionSegmentInfo,
    pub(crate) mmr: bool,
    pub(crate) template: Template,
    pub(crate) tpgdon: bool,
    pub(crate) adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) data: &'a [u8],
}

/// Parse a generic region segment header (7.4.6.1).
pub(crate) fn parse<'a>(
    reader: &mut Reader<'a>,
    had_unknown_length: bool,
) -> Result<GenericRegionHeader<'a>> {
    let mut region_info = parse_region_segment_info(reader)?;
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let mmr = flags & 0x01 != 0;
    let template = Template::from_byte(flags >> 1);
    let tpgdon = flags & 0x08 != 0;
    let ext_template = flags & 0x10 != 0;
    let adaptive_template_pixels = if mmr {
        Vec::new()
    } else {
        parse_adaptive_template_pixels(reader, template, ext_template)?
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
    })
}

/// Parse adaptive template pixel positions (7.4.6.3).
pub(crate) fn parse_adaptive_template_pixels(
    reader: &mut Reader<'_>,
    template: Template,
    // TODO: Find a test with this flag.
    _ext_template: bool,
) -> Result<Vec<AdaptiveTemplatePixel>> {
    let num_pixels = template.adaptive_template_pixels() as usize;

    let mut pixels = Vec::with_capacity(num_pixels);

    for _ in 0..num_pixels {
        let x = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;
        let y = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;

        // Validate AT pixel location (6.2.5.4, Figure 7).
        // AT pixels must reference already-decoded pixels:
        // - y must be <= 0 (current row or above)
        // - if y == 0, x must be < 0 (strictly to the left of current pixel)
        if y > 0 || (y == 0 && x >= 0) {
            bail!(TemplateError::InvalidAtPixel);
        }

        pixels.push(AdaptiveTemplatePixel { x, y });
    }

    Ok(pixels)
}

/// Whether the adaptive template pixels correspond to the default ones.
/// See Table 5.
fn has_default_at_pixels(template: Template, at_pixels: &[AdaptiveTemplatePixel]) -> bool {
    match template {
        Template::Template0 => {
            at_pixels.len() == 4
                && at_pixels[0].x == 3
                && at_pixels[0].y == -1
                && at_pixels[1].x == -3
                && at_pixels[1].y == -1
                && at_pixels[2].x == 2
                && at_pixels[2].y == -2
                && at_pixels[3].x == -2
                && at_pixels[3].y == -2
        }
        Template::Template1 => at_pixels.len() == 1 && at_pixels[0].x == 3 && at_pixels[0].y == -1,
        Template::Template2 => at_pixels.len() == 1 && at_pixels[0].x == 2 && at_pixels[0].y == -1,
        Template::Template3 => at_pixels.len() == 1 && at_pixels[0].x == 2 && at_pixels[0].y == -1,
    }
}

/// Decode a bitmap using MMR coding (6.2.6).
pub(crate) fn decode_bitmap_mmr(bitmap: &mut Bitmap, data: &[u8]) -> Result<usize> {
    /// A decoder sink that writes decoded pixels into a `Bitmap`.
    struct BitmapDecoder<'a> {
        bitmap: &'a mut Bitmap,
        x: u32,
        y: u32,
    }

    impl<'a> BitmapDecoder<'a> {
        fn new(bitmap: &'a mut Bitmap) -> Self {
            Self { bitmap, x: 0, y: 0 }
        }
    }

    impl hayro_ccitt::Decoder for BitmapDecoder<'_> {
        fn push_pixel(&mut self, white: bool) {
            if self.x < self.bitmap.width {
                self.bitmap.set_pixel(self.x, self.y, white);
                self.x += 1;
            }
        }

        fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
            const BYTE_MASKS: [u32; 4] = [0xFF000000, 0x00FF0000, 0x0000FF00, 0x000000FF];

            let row_start = (self.y * self.bitmap.stride) as usize;
            let end_x = (self.x + chunk_count * 8).min(self.bitmap.width);
            // 0xFFFFFFFF for white, 0 for black.
            let white_mask = (white as u32).wrapping_neg();

            let start = (self.x / 8) as usize;
            let end = (end_x / 8) as usize;
            let first_full = start.div_ceil(4);
            let last_full = end / 4;

            for b in start..(first_full * 4).min(end) {
                self.bitmap.data[row_start + b / 4] |= BYTE_MASKS[b % 4] & white_mask;
            }

            if last_full > first_full {
                self.bitmap.data[row_start + first_full..row_start + last_full].fill(white_mask);
            }

            for b in (first_full.max(last_full) * 4)..end {
                self.bitmap.data[row_start + b / 4] |= BYTE_MASKS[b % 4] & white_mask;
            }

            self.x = end_x;
        }

        fn next_line(&mut self) {
            self.x = 0;
            self.y += 1;
        }
    }

    let width = bitmap.width;
    let height = bitmap.height;
    let mut decoder = BitmapDecoder::new(bitmap);

    let settings = hayro_ccitt::DecodeSettings {
        columns: width,
        rows: height,
        // "If the number of bytes contained in the encoded bitmap is known in
        // advance, then it is permissible for the data stream not to contain
        // an EOFB" (6.2.6). But it _can_ contain it, which is what this
        // flag indicates.
        end_of_block: true,
        end_of_line: false,
        rows_are_byte_aligned: false,
        encoding: hayro_ccitt::EncodingMode::Group4,
        // "Pixels decoded by the MMR decoder having the value 'black' shall be
        // treated as having the value 1. Pixels decoded by the MMR decoder
        // having the value 'white' shall be treated as having the value 0."
        // (6.2.6)
        //
        // hayro-ccitt uses 1 for white, 0 for black by default, so we need to
        // invert to match JBIG2 convention.
        invert_black: true,
    };

    // "An invocation of the generic region decoding procedure with MMR equal to
    // 1 shall consume an integral number of bytes, beginning and ending on a
    // byte boundary. This may involve skipping over some bits in the last byte
    // read." (6.2.6)
    //
    // hayro-ccitt already aligns to the byte boundary before returning, so
    // nothing else to do here.
    Ok(hayro_ccitt::decode(data, &mut decoder, &settings)
        .map_err(|_| RegionError::InvalidDimension)?)
}

/// Decode a bitmap using arithmetic coding (6.2.5).
pub(crate) fn decode_bitmap_arithmetic_coding(
    bitmap: &mut Bitmap,
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [Context],
    template: Template,
    tpgdon: bool,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
) -> Result<()> {
    let width = bitmap.width;
    let height = bitmap.height;

    // "1) Set: LTP = 0" (6.2.5.7)
    let mut ltp = false;

    let mut ctx_gatherer = ContextGatherer::new(width, height, template, adaptive_template_pixels);

    // "3) Decode each row as follows:" (6.2.5.7)
    for y in 0..height {
        // "b) If TPGDON is 1, then decode a bit using the arithmetic entropy
        // coder" (6.2.5.7)
        if tpgdon {
            // See Figure 8 - 11.
            let sltp_context: u16 = match template {
                Template::Template0 => 0b1001101100100101,
                Template::Template1 => 0b0011110010101,
                Template::Template2 => 0b0011100101,
                Template::Template3 => 0b0110010101,
            };
            let sltp = decoder.decode(&mut contexts[sltp_context as usize]);
            // "Let SLTP be the value of this bit. Set: LTP = LTP XOR SLTP" (6.2.5.7)
            ltp = ltp != (sltp != 0);
        }

        // "c) If LTP = 1 then set every pixel of the current row of GBREG equal
        // to the corresponding pixel of the row immediately above." (6.2.5.7)
        if ltp {
            for x in 0..width {
                // If y == 0, pixels remain the same.
                if y > 0 {
                    let above = bitmap.get_pixel(x, y - 1);
                    bitmap.set_pixel(x, y, above);
                }
            }
        } else {
            // "d) If LTP = 0 then, from left to right, decode each pixel of the
            // current row of GBREG." (6.2.5.7)
            ctx_gatherer.start_row(bitmap, y);

            for x in 0..width {
                let context_bits = ctx_gatherer.gather(bitmap, x);
                let pixel = decoder.decode(&mut contexts[context_bits as usize]);
                let value = pixel != 0;
                bitmap.set_pixel(x, y, value);
                ctx_gatherer.update_current_row(x, value);
            }
        }
    }

    Ok(())
}

pub(crate) struct ContextGatherer<'a> {
    template: Template,
    at_pixels: &'a [AdaptiveTemplatePixel],
    use_default_at: bool,
    width: u32,
    height: u32,
    /// Current position.
    cur_y: u32,
    cur_x: u32,
    /// Pre-fetched pixel buffers for rows y-2, y-1, y.
    buf_m2: u32,
    buf_m1: u32,
    buf_cur: u32,
    /// The current context for all 3 rows.
    ctx: u16,
}

impl<'a> ContextGatherer<'a> {
    pub(crate) fn new(
        width: u32,
        height: u32,
        template: Template,
        at_pixels: &'a [AdaptiveTemplatePixel],
    ) -> Self {
        Self {
            template,
            at_pixels,
            use_default_at: has_default_at_pixels(template, at_pixels),
            width,
            height,
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

    #[inline]
    fn load_word(bitmap: &Bitmap, row_y: u32, start_x: u32) -> u32 {
        let word_idx = start_x / 32;

        if start_x.is_multiple_of(32) {
            bitmap.get_word(row_y, word_idx)
        } else {
            let bit_offset = start_x % 32;
            let word1 = bitmap.get_word(row_y, word_idx);
            let word2 = bitmap.get_word(row_y, word_idx + 1);
            (word1 << bit_offset) | (word2 >> (32 - bit_offset))
        }
    }

    #[inline]
    fn get_buf_pixel(buf: u32, pos: u32) -> u16 {
        if pos < 32 {
            ((buf >> (31 - pos)) & 1) as u16
        } else {
            0
        }
    }

    #[inline]
    fn get_bitmap_pixel(&self, bitmap: &Bitmap, px: i32, py: i32) -> u16 {
        if px < 0 || py < 0 || px >= self.width as i32 || py >= self.height as i32 {
            0
        } else if bitmap.get_pixel(px as u32, py as u32) {
            1
        } else {
            0
        }
    }

    #[inline]
    fn maybe_reload_buffers(&mut self, bitmap: &Bitmap, x: u32) {
        let max_right = match self.template {
            Template::Template0 | Template::Template1 => {
                if self.use_default_at {
                    3
                } else {
                    2
                }
            }
            Template::Template2 | Template::Template3 => {
                if self.use_default_at {
                    2
                } else {
                    1
                }
            }
        };

        if x + max_right >= self.cur_x + 32 {
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

    #[inline]
    pub(crate) fn gather(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        self.maybe_reload_buffers(bitmap, x);

        if self.use_default_at {
            match self.template {
                Template::Template0 => self.gather_template0_default(x),
                Template::Template1 => self.gather_template1_default(x),
                Template::Template2 => self.gather_template2_default(x),
                Template::Template3 => self.gather_template3_default(x),
            }
        } else {
            match self.template {
                Template::Template0 => self.gather_template0_custom(bitmap, x),
                Template::Template1 => self.gather_template1_custom(bitmap, x),
                Template::Template2 => self.gather_template2_custom(bitmap, x),
                Template::Template3 => self.gather_template3_custom(bitmap, x),
            }
        }
    }

    #[inline]
    fn gather_template0_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 12)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 5)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | (self.get_bitmap_pixel(
                bitmap,
                xi + self.at_pixels[3].x as i32,
                yi + self.at_pixels[3].y as i32,
            ) << 15)
            | (self.get_bitmap_pixel(
                bitmap,
                xi + self.at_pixels[2].x as i32,
                yi + self.at_pixels[2].y as i32,
            ) << 11)
            | (self.get_bitmap_pixel(
                bitmap,
                xi + self.at_pixels[1].x as i32,
                yi + self.at_pixels[1].y as i32,
            ) << 10)
            | (self.get_bitmap_pixel(
                bitmap,
                xi + self.at_pixels[0].x as i32,
                yi + self.at_pixels[0].y as i32,
            ) << 4);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T0) | new_pixels;
        self.ctx
    }

    #[inline]
    fn gather_template1_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 9)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | (self.get_bitmap_pixel(
                bitmap,
                xi + self.at_pixels[0].x as i32,
                yi + self.at_pixels[0].y as i32,
            ) << 3);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T1) | new_pixels;
        self.ctx
    }

    #[inline]
    fn gather_template2_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 7)
            | (Self::get_buf_pixel(self.buf_m1, bx + 1) << 3)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | (self.get_bitmap_pixel(
                bitmap,
                xi + self.at_pixels[0].x as i32,
                yi + self.at_pixels[0].y as i32,
            ) << 2);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T2) | new_pixels;
        self.ctx
    }

    #[inline]
    fn gather_template3_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m1, bx + 1) << 5)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | (self.get_bitmap_pixel(
                bitmap,
                xi + self.at_pixels[0].x as i32,
                yi + self.at_pixels[0].y as i32,
            ) << 4);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T3) | new_pixels;
        self.ctx
    }

    #[inline]
    fn gather_template0_default(&mut self, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 11)
            | (Self::get_buf_pixel(self.buf_m1, bx + 3) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T0_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline]
    fn gather_template1_default(&mut self, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 9)
            | (Self::get_buf_pixel(self.buf_m1, bx + 3) << 3)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T1_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline]
    fn gather_template2_default(&mut self, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 7)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 2)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T2_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline]
    fn gather_template3_default(&mut self, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m1, bx + 2) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T3_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline]
    pub(crate) fn update_current_row(&mut self, x: u32, value: bool) {
        if x >= self.cur_x && x < self.cur_x + 32 {
            let bit_pos = 31 - (x - self.cur_x);
            if value {
                self.buf_cur |= 1 << bit_pos;
            } else {
                self.buf_cur &= !(1 << bit_pos);
            }
        }
    }
}
