use super::{AdaptiveTemplatePixel, Template, generic};
use crate::DecodeState;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::{Bitmap, WORD_BITS, Word};
use crate::cancel::{WORD_WORK, WorkCounter};
use crate::error::{OverflowError, ParseError, Result};
use crate::reader::Reader;
use alloc::vec::Vec;

pub(crate) fn decode(
    header: &PatternDictionaryHeader<'_>,
    scratch: &mut DecodeState<'_>,
) -> Result<PatternDictionary> {
    let pattern_width = header.pattern_width as u32;
    let pattern_height = header.pattern_height as u32;
    let num_patterns = header
        .num_patterns
        .checked_add(1)
        .ok_or(OverflowError::BitmapDimension)?;

    let collective_width = num_patterns
        .checked_mul(pattern_width)
        .ok_or(OverflowError::BitmapDimension)?;

    let mut collective_bitmap = Bitmap::new(collective_width, pattern_height, &mut scratch.budget)?;

    if header.mmr {
        let _ = generic::decode_bitmap_mmr(
            &mut collective_bitmap,
            header.data,
            header.strict,
            scratch.cancel,
        )?;
    } else {
        let at_pixels = match header.template {
            Template::Template0 => [
                AdaptiveTemplatePixel {
                    x: -(pattern_width as i8),
                    y: 0,
                },
                AdaptiveTemplatePixel { x: -3, y: -1 },
                AdaptiveTemplatePixel { x: 2, y: -2 },
                AdaptiveTemplatePixel { x: -2, y: -2 },
            ],
            Template::Template1 | Template::Template2 | Template::Template3 => [
                AdaptiveTemplatePixel {
                    x: -(pattern_width as i8),
                    y: 0,
                },
                AdaptiveTemplatePixel::default(),
                AdaptiveTemplatePixel::default(),
                AdaptiveTemplatePixel::default(),
            ],
        };

        let mut decoder = ArithmeticDecoder::new(header.data);
        scratch.contexts.clear();
        scratch.contexts.resize(
            1 << header.template.context_bits(),
            ArithmeticDecoderContext::default(),
        );

        generic::decode_bitmap_arithmetic_coding(
            &mut collective_bitmap,
            &mut decoder,
            &mut scratch.contexts,
            header.template,
            false,
            &at_pixels,
            scratch.cancel,
        )?;
    }

    let mut patterns = Vec::with_capacity(num_patterns as usize);

    let src_stride = collective_bitmap.stride;

    let mut work = WorkCounter::new();
    let pattern_work = 1 + u64::from(WORD_WORK)
        * u64::from(pattern_height)
        * u64::from(pattern_width.div_ceil(WORD_BITS));

    for gray in 0..num_patterns {
        work.add(pattern_work, scratch.cancel)?;

        let start_x = gray * pattern_width;
        let mut pattern = Bitmap::new(pattern_width, pattern_height, &mut scratch.budget)?;

        let src_word_idx = start_x / WORD_BITS;
        let src_bit_offset = start_x % WORD_BITS;
        let dst_stride = pattern.stride;

        for y in 0..pattern_height {
            let src_row = (y * src_stride + src_word_idx) as usize;
            let dst_row = (y * dst_stride) as usize;

            for w in 0..dst_stride {
                let s1 = collective_bitmap.data[src_row + w as usize];
                let s2 = collective_bitmap
                    .data
                    .get(src_row + w as usize + 1)
                    .copied()
                    .unwrap_or(0);

                let word: Word = if src_bit_offset == 0 {
                    s1
                } else {
                    (s1 << src_bit_offset) | (s2 >> (WORD_BITS - src_bit_offset))
                };

                pattern.data[dst_row + w as usize] = word;
            }

            let trailing = pattern_width % WORD_BITS;
            if trailing != 0 {
                let last = dst_row + (dst_stride - 1) as usize;
                pattern.data[last] &= !0 << (WORD_BITS - trailing);
            }
        }

        patterns.push(pattern);
    }

    Ok(PatternDictionary {
        patterns,
        pattern_width,
        pattern_height,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct PatternDictionary {
    pub(crate) patterns: Vec<Bitmap>,
    pub(crate) pattern_width: u32,
    pub(crate) pattern_height: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct PatternDictionaryHeader<'a> {
    pub(crate) mmr: bool,
    pub(crate) template: Template,
    pub(crate) pattern_width: u8,
    pub(crate) pattern_height: u8,
    pub(crate) num_patterns: u32,
    pub(crate) data: &'a [u8],
    pub(crate) strict: bool,
}

pub(crate) fn parse<'a>(
    reader: &mut Reader<'a>,
    strict: bool,
) -> Result<PatternDictionaryHeader<'a>> {
    let flags_byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let mmr = flags_byte & 0x01 != 0;

    let template = Template::from_byte(flags_byte >> 1);
    let pattern_width = reader
        .read_nonzero_byte()
        .ok_or(ParseError::UnexpectedEof)?;
    let pattern_height = reader
        .read_nonzero_byte()
        .ok_or(ParseError::UnexpectedEof)?;
    let num_patterns = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    Ok(PatternDictionaryHeader {
        mmr,
        template,
        pattern_width,
        pattern_height,
        num_patterns,
        data,
        strict,
    })
}
