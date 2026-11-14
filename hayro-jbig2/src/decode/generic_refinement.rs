use alloc::vec::Vec;

use super::{
    AdaptiveTemplatePixel, RefinementTemplate, RegionBitmap, RegionSegmentInfo,
    parse_refinement_at_pixels, parse_region_segment_info,
};
use crate::DecodeState;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::{Bitmap, MAX_DIMENSION, WORD_BITS, WORD_SHIFT, Word};
use crate::cancel::{Cancel, row_mask};
use crate::decode::generic::ContextGatherer;
use crate::error::{OverflowError, ParseError, RegionError, Result, bail};
use crate::reader::Reader;

pub(crate) fn decode(
    header: &GenericRefinementRegionHeader<'_>,
    reference: &Bitmap,
    ctx: &mut DecodeState<'_>,
) -> Result<RegionBitmap> {
    let mut region = Bitmap::new_with(
        header.region_info.width,
        header.region_info.height,
        header.region_info.x_location,
        header.region_info.y_location,
        false,
        &mut ctx.budget,
    )?;

    decode_into(header, reference, &mut region, ctx)?;

    Ok(RegionBitmap {
        bitmap: region,
        combination_operator: header.region_info.combination_operator,
    })
}

pub(crate) fn decode_into(
    header: &GenericRefinementRegionHeader<'_>,
    reference: &Bitmap,
    region: &mut Bitmap,
    ctx: &mut DecodeState<'_>,
) -> Result<()> {
    let data = header.data;

    if header.region_info.width > reference.width || header.region_info.height > reference.height {
        bail!(RegionError::InvalidDimension);
    }

    let reference_dx = i32::try_from(reference.x_location)
        .ok()
        .and_then(|r: i32| {
            i32::try_from(header.region_info.x_location)
                .ok()
                .and_then(|h| r.checked_sub(h))
        })
        .ok_or(OverflowError::ReferenceOffset)?;
    let reference_dy = i32::try_from(reference.y_location)
        .ok()
        .and_then(|r: i32| {
            i32::try_from(header.region_info.y_location)
                .ok()
                .and_then(|h| r.checked_sub(h))
        })
        .ok_or(OverflowError::ReferenceOffset)?;

    let mut decoder = ArithmeticDecoder::new(data);
    let num_context_bits = header.template.context_bits();
    ctx.contexts.clear();
    ctx.contexts
        .resize(1 << num_context_bits, ArithmeticDecoderContext::default());

    decode_bitmap(
        &mut decoder,
        &mut ctx.contexts,
        region,
        reference,
        reference_dx,
        reference_dy,
        header.template,
        &header.adaptive_template_pixels,
        header.tpgron,
        ctx.cancel,
    )?;

    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct GenericRefinementRegionHeader<'a> {
    pub(crate) region_info: RegionSegmentInfo,
    pub(crate) template: RefinementTemplate,
    pub(crate) tpgron: bool,
    pub(crate) adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) data: &'a [u8],
}

pub(crate) fn parse<'a>(reader: &mut Reader<'a>) -> Result<GenericRefinementRegionHeader<'a>> {
    let region_info = parse_region_segment_info(reader)?;
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let template = RefinementTemplate::from_byte(flags);
    let tpgron = flags & 0x02 != 0;
    let adaptive_template_pixels = if template == RefinementTemplate::Template0 {
        parse_refinement_at_pixels(reader)?
    } else {
        Vec::new()
    };
    let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    Ok(GenericRefinementRegionHeader {
        region_info,
        template,
        tpgron,
        adaptive_template_pixels,
        data,
    })
}

fn has_default_at_pixels(at_pixels: &[AdaptiveTemplatePixel]) -> bool {
    at_pixels.len() >= 2
        && at_pixels[0].x == -1
        && at_pixels[0].y == -1
        && at_pixels[1].x == -1
        && at_pixels[1].y == -1
}

pub(crate) struct RefinementContextGatherer<'a> {
    template: RefinementTemplate,
    at_pixels: &'a [AdaptiveTemplatePixel],
    use_default_at: bool,
    reference_dx: i32,
    reference_dy: i32,
    reg_y: u32,
    reg_cur_x: u32,
    reg_m1: Word,
    reg_cur: Word,
    ref_cur_x: i32,
    ref_m1: Word,
    ref_cur: Word,
    ref_p1: Word,
    ctx: u16,
}

const SHIFT_MASK_T0_DEFAULT: u16 = 0b1_1001_1011_0110;
const SHIFT_MASK_T0_CUSTOM: u16 = 0b0_1000_1011_0110;
const SHIFT_MASK_T1: u16 = 0b11_0001_1010;

impl<'a> RefinementContextGatherer<'a> {
    pub(crate) fn new(
        template: RefinementTemplate,
        at_pixels: &'a [AdaptiveTemplatePixel],
        reference_dx: i32,
        reference_dy: i32,
    ) -> Self {
        // Larger offsets put all reference pixels outside the bitmap; clamping keeps coordinates in `i32`.
        const MAX_OFFSET: i32 = 4 * MAX_DIMENSION as i32;

        let use_default_at =
            template == RefinementTemplate::Template0 && has_default_at_pixels(at_pixels);
        Self {
            template,
            at_pixels,
            use_default_at,
            reference_dx: reference_dx.clamp(-MAX_OFFSET, MAX_OFFSET),
            reference_dy: reference_dy.clamp(-MAX_OFFSET, MAX_OFFSET),
            reg_y: 0,
            reg_cur_x: 0,
            reg_m1: 0,
            reg_cur: 0,
            ref_cur_x: 0,
            ref_m1: 0,
            ref_cur: 0,
            ref_p1: 0,
            ctx: 0,
        }
    }

    pub(crate) fn start_row(&mut self, region: &Bitmap, reference: &Bitmap, y: u32) {
        self.reg_y = y;

        self.reg_cur_x = 0;
        self.reg_m1 = if y >= 1 {
            ContextGatherer::load_word(region, y - 1, 0)
        } else {
            0
        };
        self.reg_cur = 0;

        self.load_reference_words(reference, -self.reference_dx - 1);
        let rbx = 1;

        self.ctx = match self.template {
            RefinementTemplate::Template1 => {
                let reg_m1_0 = ContextGatherer::get_buf_pixel(self.reg_m1, 0);
                let ref_cur_m1 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx - 1);
                let ref_cur_0 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx);
                let ref_p1_0 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx);

                (reg_m1_0 << 7) | (ref_cur_m1 << 3) | (ref_cur_0 << 2) | (ref_p1_0)
            }
            RefinementTemplate::Template0 if self.use_default_at => {
                let reg_m1_0 = ContextGatherer::get_buf_pixel(self.reg_m1, 0);
                let ref_m1_m1 = ContextGatherer::get_buf_pixel(self.ref_m1, rbx - 1);
                let ref_m1_0 = ContextGatherer::get_buf_pixel(self.ref_m1, rbx);
                let ref_cur_m1 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx - 1);
                let ref_cur_0 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx);
                let ref_p1_m1 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx - 1);
                let ref_p1_0 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx);

                (reg_m1_0 << 10)
                    | (ref_m1_m1 << 7)
                    | (ref_m1_0 << 6)
                    | (ref_cur_m1 << 4)
                    | (ref_cur_0 << 3)
                    | (ref_p1_m1 << 1)
                    | ref_p1_0
            }
            RefinementTemplate::Template0 => {
                let reg_m1_0 = ContextGatherer::get_buf_pixel(self.reg_m1, 0);
                let ref_m1_0 = ContextGatherer::get_buf_pixel(self.ref_m1, rbx);
                let ref_cur_m1 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx - 1);
                let ref_cur_0 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx);
                let ref_p1_m1 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx - 1);
                let ref_p1_0 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx);
                (reg_m1_0 << 10)
                    | (ref_m1_0 << 6)
                    | (ref_cur_m1 << 4)
                    | (ref_cur_0 << 3)
                    | (ref_p1_m1 << 1)
                    | ref_p1_0
            }
        };
    }

    #[inline(always)]
    pub(crate) fn maybe_reload_buffers(&mut self, region: &Bitmap, reference: &Bitmap, x: u32) {
        if x + 1 >= self.reg_cur_x + WORD_BITS {
            let new_start = x.saturating_sub(1);
            self.reg_cur_x = new_start;
            self.reg_m1 = if self.reg_y >= 1 {
                ContextGatherer::load_word(region, self.reg_y - 1, new_start)
            } else {
                0
            };
            self.reg_cur = ContextGatherer::load_word(region, self.reg_y, new_start);
        }

        let ref_x = x as i32 - self.reference_dx;
        if ref_x + 1 >= self.ref_cur_x + WORD_BITS as i32 {
            self.load_reference_words(reference, ref_x - 1);
        }
    }

    fn load_reference_words(&mut self, reference: &Bitmap, start: i32) {
        let ref_y = self.reg_y as i32 - self.reference_dy;
        let load = |row: i32| {
            // Negative rows wrap to large values, which lie outside the bitmap.
            let row = row as u32;
            if start >= 0 {
                ContextGatherer::load_word(reference, row, start as u32)
            } else if start > -(WORD_BITS as i32) {
                ContextGatherer::load_word(reference, row, 0) >> start.unsigned_abs()
            } else {
                0
            }
        };

        self.ref_cur_x = start;
        self.ref_m1 = load(ref_y - 1);
        self.ref_cur = load(ref_y);
        self.ref_p1 = load(ref_y + 1);
    }

    #[inline(always)]
    fn reference_buffer_x(&self, x: u32) -> u32 {
        (x as i32 - self.reference_dx - self.ref_cur_x) as u32
    }

    #[inline(always)]
    pub(crate) fn tpgr_all_same(&self, x: u32) -> bool {
        let rbx = self.reference_buffer_x(x);

        // The buffers are reloaded so that the pixels left and right of `rbx` always lie within them.
        #[inline(always)]
        fn extract_3bits(buf: Word, rbx: u32) -> u32 {
            #[allow(trivial_numeric_casts)]
            #[allow(clippy::unnecessary_cast)]
            {
                ((buf >> (WORD_SHIFT - rbx - 1)) & 0b111) as u32
            }
        }

        let m1 = extract_3bits(self.ref_m1, rbx);
        let cur = extract_3bits(self.ref_cur, rbx);
        let p1 = extract_3bits(self.ref_p1, rbx);

        m1 == cur && cur == p1 && (cur == 0 || cur == 0b111)
    }

    #[inline(always)]
    pub(crate) fn ref_center_pixel(&self, x: u32) -> u8 {
        let rbx = self.reference_buffer_x(x);

        ContextGatherer::get_buf_pixel(self.ref_cur, rbx) as u8
    }

    #[inline(always)]
    pub(crate) fn update_current_row(&mut self, x: u32, value: u8) {
        let bx = x - self.reg_cur_x;
        debug_assert!(bx < WORD_BITS);
        let bit_pos = WORD_SHIFT - bx;
        #[allow(trivial_numeric_casts)]
        let mask = (1 as Word) << bit_pos;
        self.reg_cur = (self.reg_cur & !mask) | ((value as Word) << bit_pos);
    }

    #[inline(always)]
    fn gather_template0_default(&mut self, _region: &Bitmap, _reference: &Bitmap, x: u32) -> u16 {
        let bx = x - self.reg_cur_x;
        let rbx = self.reference_buffer_x(x);

        let new_pixels = (ContextGatherer::get_buf_pixel(self.reg_m1, bx + 1) << 10)
            | (ContextGatherer::get_buf_pixel(self.reg_cur, bx.wrapping_sub(1)) << 9)
            | (ContextGatherer::get_buf_pixel(self.ref_m1, rbx + 1) << 6)
            | (ContextGatherer::get_buf_pixel(self.ref_cur, rbx + 1) << 3)
            | ContextGatherer::get_buf_pixel(self.ref_p1, rbx + 1);

        self.ctx = ((self.ctx << 1) & SHIFT_MASK_T0_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template0_custom(&mut self, region: &Bitmap, reference: &Bitmap, x: u32) -> u16 {
        let bx = x - self.reg_cur_x;
        let ref_x_i = x as i32 - self.reference_dx;
        let rbx = self.reference_buffer_x(x);

        let xi = x as i32;
        let yi = self.reg_y as i32;
        let ref_y = yi - self.reference_dy;
        let at1 = self.at_pixels[0];
        let at2 = self.at_pixels[1];

        let new_pixels = ((region.get_pixel((xi + at1.x as i32) as u32, (yi + at1.y as i32) as u32)
            as u16)
            << 12)
            | (ContextGatherer::get_buf_pixel(self.reg_m1, bx + 1) << 10)
            | (ContextGatherer::get_buf_pixel(self.reg_cur, bx.wrapping_sub(1)) << 9)
            | ((reference.get_pixel(
                (ref_x_i + at2.x as i32) as u32,
                (ref_y + at2.y as i32) as u32,
            ) as u16)
                << 8)
            | (ContextGatherer::get_buf_pixel(self.ref_m1, rbx + 1) << 6)
            | (ContextGatherer::get_buf_pixel(self.ref_cur, rbx + 1) << 3)
            | ContextGatherer::get_buf_pixel(self.ref_p1, rbx + 1);

        self.ctx = ((self.ctx << 1) & SHIFT_MASK_T0_CUSTOM) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template1(&mut self, _region: &Bitmap, _reference: &Bitmap, x: u32) -> u16 {
        let bx = x - self.reg_cur_x;
        let rbx = self.reference_buffer_x(x);

        let new_pixels = (ContextGatherer::get_buf_pixel(self.reg_m1, bx + 1) << 7)
            | (ContextGatherer::get_buf_pixel(self.reg_cur, bx.wrapping_sub(1)) << 6)
            | (ContextGatherer::get_buf_pixel(self.ref_m1, rbx) << 5)
            | (ContextGatherer::get_buf_pixel(self.ref_cur, rbx + 1) << 2)
            | ContextGatherer::get_buf_pixel(self.ref_p1, rbx + 1);

        self.ctx = ((self.ctx << 1) & SHIFT_MASK_T1) | new_pixels;
        self.ctx
    }
}

pub(crate) fn decode_bitmap(
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [ArithmeticDecoderContext],
    region: &mut Bitmap,
    reference: &Bitmap,
    reference_dx: i32,
    reference_dy: i32,
    gr_template: RefinementTemplate,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
    tpgron: bool,
    cancel: Cancel<'_>,
) -> Result<()> {
    macro_rules! refinement_decode_loop {
        ($gatherer:expr, $tpgron:expr, $sltp_context:expr, $gather:expr) => {{
            let gatherer: &mut RefinementContextGatherer<'_> = $gatherer;
            let width = region.width;
            let height = region.height;

            let mut ltp = false;
            let check_mask = row_mask(width);

            for y in 0..height {
                if decoder.is_exhausted() {
                    bail!(ParseError::UnexpectedEof);
                }

                if y & check_mask == check_mask {
                    cancel.check()?;
                }

                if $tpgron {
                    let sltp = decoder.read_bit(&mut contexts[$sltp_context as usize]);
                    ltp = ltp != (sltp != 0);
                }

                gatherer.start_row(region, reference, y);

                if !ltp {
                    for x in 0..width {
                        gatherer.maybe_reload_buffers(region, reference, x);
                        let context = ($gather)(gatherer, region, reference, x) as usize;
                        let pixel = decoder.read_bit(&mut contexts[context]);
                        let value = pixel as u8;

                        if value != 0 {
                            region.set_pixel(x, y, value);
                            gatherer.update_current_row(x, value);
                        }
                    }
                } else {
                    for x in 0..width {
                        gatherer.maybe_reload_buffers(region, reference, x);

                        let context = ($gather)(gatherer, region, reference, x) as usize;

                        if gatherer.tpgr_all_same(x) {
                            let val = gatherer.ref_center_pixel(x);

                            if val != 0 {
                                region.set_pixel(x, y, val);
                                gatherer.update_current_row(x, val);
                            }
                        } else {
                            let pixel = decoder.read_bit(&mut contexts[context]);
                            let value = pixel as u8;

                            if value != 0 {
                                region.set_pixel(x, y, value);
                                gatherer.update_current_row(x, value);
                            }
                        }
                    }
                }
            }
        }};
    }

    let mut gatherer = RefinementContextGatherer::new(
        gr_template,
        adaptive_template_pixels,
        reference_dx,
        reference_dy,
    );

    let sltp_context: u16 = match gr_template {
        RefinementTemplate::Template0 => 0b0000000010000,
        RefinementTemplate::Template1 => 0b0000001000,
    };

    match gr_template {
        RefinementTemplate::Template0 if gatherer.use_default_at => refinement_decode_loop!(
            &mut gatherer,
            tpgron,
            sltp_context,
            RefinementContextGatherer::gather_template0_default
        ),
        RefinementTemplate::Template0 => refinement_decode_loop!(
            &mut gatherer,
            tpgron,
            sltp_context,
            RefinementContextGatherer::gather_template0_custom
        ),
        RefinementTemplate::Template1 => refinement_decode_loop!(
            &mut gatherer,
            tpgron,
            sltp_context,
            RefinementContextGatherer::gather_template1
        ),
    }

    Ok(())
}
