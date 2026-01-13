//! Region segment information field parsing (7.4.1).

pub(crate) mod generic;
pub(crate) mod generic_refinement;
pub(crate) mod halftone;
pub(crate) mod pattern;
pub(crate) mod symbol;
pub(crate) mod text;

use crate::error::{ParseError, RegionError, Result, bail, err};
use crate::reader::Reader;

/// "These operators describe how the segment's bitmap is to be combined with
/// the page bitmap." (7.4.1.5)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CombinationOperator {
    /// 0 OR
    Or,
    /// 1 AND
    And,
    /// 2 XOR
    Xor,
    /// 3 XNOR
    Xnor,
    /// 4 REPLACE
    Replace,
}

impl CombinationOperator {
    fn from_value(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Or),
            1 => Ok(Self::And),
            2 => Ok(Self::Xor),
            3 => Ok(Self::Xnor),
            4 => Ok(Self::Replace),
            _ => err!(RegionError::InvalidCombinationOperator),
        }
    }
}

/// Parsed region segment information field (7.4.1).
///
/// "A region segment information field contains the following subfields, as
/// shown in Figure 30:
/// - Region segment bitmap width – see 7.4.1.1.
/// - Region segment bitmap height – see 7.4.1.2.
/// - Region segment bitmap X location – see 7.4.1.3.
/// - Region segment bitmap Y location – see 7.4.1.4.
/// - Region segment flags – see 7.4.1.5." (7.4.1)
#[derive(Debug, Clone)]
pub(crate) struct RegionSegmentInfo {
    /// "This four-byte field gives the width in pixels of the bitmap encoded
    /// in this segment." (7.4.1.1)
    pub(crate) width: u32,
    /// "This four-byte field gives the height in pixels of the bitmap encoded
    /// in this segment." (7.4.1.2)
    pub(crate) height: u32,
    /// "This four-byte field gives the horizontal offset in pixels of the bitmap
    /// encoded in this segment relative to the page bitmap." (7.4.1.3)
    pub(crate) x_location: u32,
    /// "This four-byte field gives the vertical offset in pixels of the bitmap
    /// encoded in this segment relative to the page bitmap." (7.4.1.4)
    pub(crate) y_location: u32,
    /// "Bits 0-2: External combination operator." (7.4.1.5)
    pub(crate) combination_operator: CombinationOperator,
    /// "Bit 3: Colour extension flag (COLEXTFLAG). This field specifies whether
    /// the region segment is extended to represent coloured bitmap." (7.4.1.5)
    pub(crate) _colour_extension: bool,
}

/// Parse the region segment information field (7.4.1).
pub(crate) fn parse_region_segment_info(reader: &mut Reader<'_>) -> Result<RegionSegmentInfo> {
    // 7.4.1.1: Region segment bitmap width
    let width = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    // 7.4.1.2: Region segment bitmap height
    let height = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    // 7.4.1.3: Region segment bitmap X location
    let x_location = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    // 7.4.1.: Region segment bitmap Y location
    let y_location = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    // 7.4.1.5: Region segment flags
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    // "Bits 0-2: External combination operator."
    let combination_operator = CombinationOperator::from_value(flags & 0x07)?;

    // "Bit 3: Colour extension flag (COLEXTFLAG)."
    let colour_extension = flags & 0x08 != 0;

    // "Bits 4-7: Reserved; must be 0."
    if flags & 0xF0 != 0 {
        bail!(RegionError::InvalidCombinationOperator);
    }

    Ok(RegionSegmentInfo {
        width,
        height,
        x_location,
        y_location,
        combination_operator,
        _colour_extension: colour_extension,
    })
}
