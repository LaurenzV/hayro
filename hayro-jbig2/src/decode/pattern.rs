//! Pattern dictionary segment parsing and decoding (7.4.4, 6.7).

use alloc::vec;
use alloc::vec::Vec;

use super::{AdaptiveTemplatePixel, CombinationOperator, Template, generic};
use crate::bitmap::DecodedRegion;
use crate::error::{DecodeError, ParseError, Result};
use crate::reader::Reader;

/// Decode a pattern dictionary segment (7.4.4.2, 6.7).
pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<PatternDictionary> {
    let header = parse(reader)?;

    let pattern_width = header.pattern_width as u32;
    let pattern_height = header.pattern_height as u32;
    let num_patterns = header
        .num_patterns
        .checked_add(1)
        .ok_or(DecodeError::Overflow)?;

    // "1) Create a bitmap B_HDC. The height of this bitmap is HDPH. The width
    // of the bitmap is (GRAYMAX + 1) × HDPW. This bitmap contains all the
    // patterns concatenated left to right." (6.7.5)
    let collective_width = num_patterns
        .checked_mul(pattern_width)
        .ok_or(DecodeError::Overflow)?;

    let encoded_data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    let mut collective_bitmap = DecodedRegion {
        width: collective_width,
        height: pattern_height,
        data: vec![false; (collective_width * pattern_height) as usize],
        x_location: 0,
        y_location: 0,
        combination_operator: CombinationOperator::Replace,
    };

    // "2) Decode the collective bitmap using a generic region decoding procedure
    // as described in 6.2." (6.7.5)
    if header.mmr {
        let _ = generic::decode_bitmap_mmr(&mut collective_bitmap, encoded_data)?;
    } else {
        let at_pixels = match header.template {
            Template::Template0 => {
                vec![
                    AdaptiveTemplatePixel {
                        x: -(pattern_height as i8),
                        y: 0,
                    },
                    AdaptiveTemplatePixel { x: -3, y: -1 },
                    AdaptiveTemplatePixel { x: 2, y: -2 },
                    AdaptiveTemplatePixel { x: -2, y: -2 },
                ]
            }
            Template::Template1 | Template::Template2 | Template::Template3 => {
                vec![AdaptiveTemplatePixel {
                    x: -(pattern_width as i8),
                    y: 0,
                }]
            }
        };

        generic::decode_bitmap_arithmetic_coding(
            &mut collective_bitmap,
            encoded_data,
            header.template,
            false,
            &at_pixels,
        )?;
    }

    // "3) Set: GRAY = 0" (6.7.5)
    // "4) While GRAY ≤ GRAYMAX:" (6.7.5)
    let mut patterns = Vec::with_capacity(num_patterns as usize);

    for gray in 0..num_patterns {
        // "a) Let the subimage of B_HDC consisting of HPH rows and columns
        // HDPW × GRAY through HDPW × (GRAY + 1) − 1 be denoted B_P. Set:
        // HDPATS[GRAY] = B_P" (6.7.5)"
        let start_x = gray * pattern_width;
        let pattern = {
            let mut pattern = DecodedRegion::new(pattern_width, pattern_height);

            for y in 0..pattern_height {
                for x in 0..pattern_width {
                    let pixel = collective_bitmap.get_pixel(start_x + x, y);
                    pattern.set_pixel(x, y, pixel);
                }
            }

            pattern
        };

        patterns.push(pattern);
    }

    Ok(PatternDictionary {
        patterns,
        pattern_width,
        pattern_height,
    })
}

/// A decoded pattern dictionary.
#[derive(Debug, Clone)]
pub(crate) struct PatternDictionary {
    pub(crate) patterns: Vec<DecodedRegion>,
    pub(crate) pattern_width: u32,
    pub(crate) pattern_height: u32,
}

/// Parsed pattern dictionary segment header (7.4.4.1).
#[derive(Debug, Clone)]
struct PatternDictionaryHeader {
    mmr: bool,
    template: Template,
    /// `HDPW`
    pattern_width: u8,
    /// `HDPH`
    pattern_height: u8,
    /// `GRAYMAX`
    num_patterns: u32,
}

/// Parse a pattern dictionary segment header (7.4.4.1).
fn parse(reader: &mut Reader<'_>) -> Result<PatternDictionaryHeader> {
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

    Ok(PatternDictionaryHeader {
        mmr,
        template,
        pattern_width,
        pattern_height,
        num_patterns,
    })
}
