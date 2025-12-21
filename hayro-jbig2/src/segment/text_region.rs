//! Text region segment parsing (7.4.3).
//!
//! "The data parts of all three of the text region segment types ('intermediate
//! text region', 'immediate text region' and 'immediate lossless text region')
//! are coded identically, but are acted upon differently, see 8.2. The syntax
//! of these segment types' data parts is specified here." (7.4.3)

use crate::reader::Reader;
use crate::segment::generic_refinement_region::RefinementAdaptiveTemplatePixel;
use crate::segment::region::{CombinationOperator, RegionSegmentInfo, parse_region_segment_info};

/// Reference corner for symbol placement (REFCORNER).
///
/// "Bits 4-5: REFCORNER. The four values that this two-bit field can take are:
/// 0 BOTTOMLEFT
/// 1 TOPLEFT
/// 2 BOTTOMRIGHT
/// 3 TOPRIGHT" (7.4.3.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceCorner {
    /// "0 BOTTOMLEFT"
    BottomLeft,
    /// "1 TOPLEFT"
    TopLeft,
    /// "2 BOTTOMRIGHT"
    BottomRight,
    /// "3 TOPRIGHT"
    TopRight,
}

impl ReferenceCorner {
    fn from_value(value: u8) -> Self {
        match value {
            0 => Self::BottomLeft,
            1 => Self::TopLeft,
            2 => Self::BottomRight,
            3 => Self::TopRight,
            _ => unreachable!(),
        }
    }
}

/// Parsed text region segment flags (7.4.3.1.1).
///
/// "This two-byte field is formatted as shown in Figure 38 and as described
/// below." (7.4.3.1.1)
#[derive(Debug, Clone)]
pub(crate) struct TextRegionFlags {
    /// "Bit 0: SBHUFF. If this bit is 1, then the segment uses the Huffman
    /// encoding variant. If this bit is 0, then the segment uses the arithmetic
    /// encoding variant. The setting of this flag determines how the data in
    /// this segment are encoded." (7.4.3.1.1)
    pub sbhuff: bool,

    /// "Bit 1: SBREFINE. If this bit is 0, then the segment contains no symbol
    /// instance refinements. If this bit is 1, then the segment may contain
    /// symbol instance refinements." (7.4.3.1.1)
    pub sbrefine: bool,

    /// "Bits 2-3: LOGSBSTRIPS. This two-bit field codes the base-2 logarithm of
    /// the strip size used to encode the segment. Thus, strip sizes of 1, 2, 4,
    /// and 8 can be encoded." (7.4.3.1.1)
    pub log_sb_strips: u8,

    /// "Bits 4-5: REFCORNER." (7.4.3.1.1)
    pub reference_corner: ReferenceCorner,

    /// "Bit 6: TRANSPOSED. If this bit is 1, then the primary direction of
    /// coding is top-to-bottom. If this bit is 0, then the primary direction
    /// of coding is left-to-right. This allows for text running up and down
    /// the page." (7.4.3.1.1)
    pub transposed: bool,

    /// "Bits 7-8: SBCOMBOP. This field has four possible values, representing
    /// one of four possible combination operators:
    /// 0 OR
    /// 1 AND
    /// 2 XOR
    /// 3 XNOR" (7.4.3.1.1)
    pub combination_operator: CombinationOperator,

    /// "Bit 9: SBDEFPIXEL. This bit contains the initial value for every pixel
    /// in the text region, before any symbols are drawn." (7.4.3.1.1)
    pub default_pixel: bool,

    /// "Bits 10-14: SBDSOFFSET. This signed five-bit field contains the value
    /// of SBDSOFFSET – see 6.4.8." (7.4.3.1.1)
    pub ds_offset: i8,

    /// "Bit 15: SBRTEMPLATE. This field controls the template used to decode
    /// symbol instance refinements if SBREFINE is 1. If SBREFINE is 0, this
    /// field must contain the value 0." (7.4.3.1.1)
    pub sbrtemplate: u8,
}

/// Parsed text region segment header (7.4.3.1).
///
/// "The data part of a text region segment begins with a text region segment
/// data header. This header contains the fields shown in Figure 37 and
/// described below." (7.4.3.1)
#[derive(Debug, Clone)]
pub(crate) struct TextRegionHeader {
    /// "Region segment information field – see 7.4.1." (7.4.3.1)
    pub region_info: RegionSegmentInfo,

    /// "Text region segment flags – see 7.4.3.1.1." (7.4.3.1)
    pub flags: TextRegionFlags,

    /// "Text region segment refinement AT flags – see 7.4.3.1.3." (7.4.3.1)
    /// "This field is only present if SBREFINE is 1 and SBRTEMPLATE is 0."
    /// Contains 2 AT pixels (4 bytes, Figure 40).
    pub refinement_at_pixels: Vec<RefinementAdaptiveTemplatePixel>,

    /// "SBNUMINSTANCES – see 7.4.3.1.4." (7.4.3.1)
    /// "This four-byte field contains the number of symbol instances coded in
    /// this segment." (7.4.3.1.4)
    pub num_instances: u32,
}

/// Parse text region segment flags (7.4.3.1.1).
fn parse_text_region_flags(reader: &mut Reader<'_>) -> Result<TextRegionFlags, &'static str> {
    let flags_word = reader.read_u16().ok_or("unexpected end of data")?;

    // "Bit 0: SBHUFF"
    let sbhuff = flags_word & 0x0001 != 0;

    // "Bit 1: SBREFINE"
    let sbrefine = flags_word & 0x0002 != 0;

    // "Bits 2-3: LOGSBSTRIPS"
    let log_sb_strips = ((flags_word >> 2) & 0x03) as u8;

    // "Bits 4-5: REFCORNER"
    let reference_corner = ReferenceCorner::from_value(((flags_word >> 4) & 0x03) as u8);

    // "Bit 6: TRANSPOSED"
    let transposed = flags_word & 0x0040 != 0;

    // "Bits 7-8: SBCOMBOP"
    let sbcombop_value = ((flags_word >> 7) & 0x03) as u8;
    let combination_operator = match sbcombop_value {
        0 => CombinationOperator::Or,
        1 => CombinationOperator::And,
        2 => CombinationOperator::Xor,
        3 => CombinationOperator::Xnor,
        _ => unreachable!(),
    };

    // "Bit 9: SBDEFPIXEL"
    let default_pixel = flags_word & 0x0200 != 0;

    // "Bits 10-14: SBDSOFFSET" (signed 5-bit field)
    let ds_offset_raw = ((flags_word >> 10) & 0x1F) as u8;
    // Sign-extend from 5 bits to i8
    let ds_offset = if ds_offset_raw & 0x10 != 0 {
        // Negative value: sign extend
        (ds_offset_raw | 0xE0) as i8
    } else {
        ds_offset_raw as i8
    };

    // "Bit 15: SBRTEMPLATE"
    let sbrtemplate = ((flags_word >> 15) & 0x01) as u8;

    Ok(TextRegionFlags {
        sbhuff,
        sbrefine,
        log_sb_strips,
        reference_corner,
        transposed,
        combination_operator,
        default_pixel,
        ds_offset,
        sbrtemplate,
    })
}

/// Parse text region refinement AT flags (7.4.3.1.3).
///
/// "This field is only present if SBREFINE is 1 and SBRTEMPLATE is 0. It is a
/// four-byte field, formatted as shown in Figure 40 and as described below."
/// (7.4.3.1.3)
fn parse_text_region_refinement_at_flags(
    reader: &mut Reader<'_>,
) -> Result<Vec<RefinementAdaptiveTemplatePixel>, &'static str> {
    let mut pixels = Vec::with_capacity(2);

    // "Byte 0: SBRATX1"
    // "Byte 1: SBRATY1"
    // "The AT coordinate X and Y fields are signed values, and may take on
    // values that are permitted according to 6.3.5.3." (7.4.3.1.3)
    let x1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x1, y: y1 });

    // "Byte 2: SBRATX2"
    // "Byte 3: SBRATY2"
    let x2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x2, y: y2 });

    Ok(pixels)
}

/// Parse a text region segment header (7.4.3.1).
pub(crate) fn parse_text_region_header(
    reader: &mut Reader<'_>,
) -> Result<TextRegionHeader, &'static str> {
    // "Region segment information field – see 7.4.1."
    let region_info = parse_region_segment_info(reader)?;

    // "Text region segment flags – see 7.4.3.1.1."
    let flags = parse_text_region_flags(reader)?;

    // Check for unsupported Huffman coding early
    if flags.sbhuff {
        return Err("SBHUFF=1 (Huffman coding) is not supported for text regions");
    }

    // "Text region segment refinement AT flags – see 7.4.3.1.3."
    // "This field is only present if SBREFINE is 1 and SBRTEMPLATE is 0."
    let refinement_at_pixels = if flags.sbrefine && flags.sbrtemplate == 0 {
        parse_text_region_refinement_at_flags(reader)?
    } else {
        Vec::new()
    };

    // "SBNUMINSTANCES – see 7.4.3.1.4."
    // "This four-byte field contains the number of symbol instances coded in
    // this segment."
    let num_instances = reader.read_u32().ok_or("unexpected end of data")?;

    Ok(TextRegionHeader {
        region_info,
        flags,
        refinement_at_pixels,
        num_instances,
    })
}
