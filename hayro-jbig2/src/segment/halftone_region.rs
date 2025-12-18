//! Halftone region segment parsing (7.4.5).

use crate::reader::Reader;
use crate::segment::region::{CombinationOperator, RegionSegmentInfo, parse_region_segment_info};

/// Template used for halftone arithmetic coding (7.4.5.1.1).
///
/// "This field controls the template used to decode halftone gray-scale value
/// bitplanes if HMMR is 0. If HMMR is 1, this field must contain the value 0."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HTemplate {
    /// Template 0
    Template0 = 0,
    /// Template 1
    Template1 = 1,
    /// Template 2
    Template2 = 2,
    /// Template 3
    Template3 = 3,
}

impl HTemplate {
    fn from_value(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(Self::Template0),
            1 => Ok(Self::Template1),
            2 => Ok(Self::Template2),
            3 => Ok(Self::Template3),
            _ => Err("invalid halftone template"),
        }
    }
}

/// Parsed halftone region segment flags (7.4.5.1.1).
///
/// "This one-byte field is formatted as shown in Figure 44."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneRegionFlags {
    /// "Bit 0: HMMR. If this bit is 1, then the segment uses the MMR encoding
    /// variant. If this bit is 0, then the segment uses the arithmetic encoding
    /// variant."
    pub hmmr: bool,
    /// "Bits 1-2: HTEMPLATE. This field controls the template used to decode
    /// halftone gray-scale value bitplanes if HMMR is 0. If HMMR is 1, this
    /// field must contain the value 0."
    pub htemplate: HTemplate,
    /// "Bit 3: HENABLESKIP. This field controls whether gray-scale values that
    /// do not contribute to the region contents are skipped during decoding.
    /// If HMMR is 1, this field must contain the value 0."
    pub henableskip: bool,
    /// "Bits 4-6: HCOMBOP. This field has five possible values, representing
    /// one of five possible combination operators."
    pub hcombop: CombinationOperator,
    /// "Bit 7: HDEFPIXEL. This bit contains the initial value for every pixel
    /// in the halftone region, before any patterns are drawn."
    pub hdefpixel: bool,
}

/// Halftone grid position and size (7.4.5.1.2).
///
/// "This field describes the location and size of the grid of gray-scale values."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneGridPositionAndSize {
    /// "HGW: This four-byte field contains the width of the array of gray-scale
    /// values." (7.4.5.1.2.1)
    pub hgw: u32,
    /// "HGH: This four-byte field contains the height of the array of gray-scale
    /// values." (7.4.5.1.2.2)
    pub hgh: u32,
    /// "HGX: This signed four-byte field contains 256 times the horizontal offset
    /// of the origin of the halftone grid." (7.4.5.1.2.3)
    pub hgx: i32,
    /// "HGY: This signed four-byte field contains 256 times the vertical offset
    /// of the origin of the halftone grid." (7.4.5.1.2.4)
    pub hgy: i32,
}

/// Halftone grid vector (7.4.5.1.3).
///
/// "This field describes the vector used to draw the grid of gray-scale values."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneGridVector {
    /// "HRX: This unsigned two-byte field contains 256 times the horizontal
    /// coordinate of the halftone grid vector." (7.4.5.1.3.1)
    pub hrx: u16,
    /// "HRY: This unsigned two-byte field contains 256 times the vertical
    /// coordinate of the halftone grid vector." (7.4.5.1.3.2)
    pub hry: u16,
}

/// Parsed halftone region segment header (7.4.5.1).
///
/// "The data part of a halftone region segment begins with a halftone region
/// segment data header. This header contains the fields shown in Figure 43."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneRegionHeader {
    /// Region segment information field (7.4.1).
    pub region_info: RegionSegmentInfo,
    /// Halftone region segment flags (7.4.5.1.1).
    pub flags: HalftoneRegionFlags,
    /// Halftone grid position and size (7.4.5.1.2).
    pub grid_position_and_size: HalftoneGridPositionAndSize,
    /// Halftone grid vector (7.4.5.1.3).
    pub grid_vector: HalftoneGridVector,
}

/// Parse a halftone region segment header (7.4.5.1).
pub(crate) fn parse_halftone_region_header(
    reader: &mut Reader<'_>,
) -> Result<HalftoneRegionHeader, &'static str> {
    // Region segment information field (7.4.1)
    let region_info = parse_region_segment_info(reader)?;

    // 7.4.5.1.1: Halftone region segment flags
    let flags_byte = reader.read_byte().ok_or("unexpected end of data")?;

    // "Bit 0: HMMR"
    let hmmr = flags_byte & 0x01 != 0;

    // "Bits 1-2: HTEMPLATE"
    let htemplate = HTemplate::from_value((flags_byte >> 1) & 0x03)?;

    // "Bit 3: HENABLESKIP"
    let henableskip = flags_byte & 0x08 != 0;

    // "Bits 4-6: HCOMBOP"
    let hcombop_value = (flags_byte >> 4) & 0x07;
    let hcombop = match hcombop_value {
        0 => CombinationOperator::Or,
        1 => CombinationOperator::And,
        2 => CombinationOperator::Xor,
        3 => CombinationOperator::Xnor,
        4 => CombinationOperator::Replace,
        _ => return Err("invalid halftone combination operator"),
    };

    // "Bit 7: HDEFPIXEL"
    let hdefpixel = flags_byte & 0x80 != 0;

    // Validate constraints when HMMR is 1
    if hmmr {
        if htemplate != HTemplate::Template0 {
            return Err("HTEMPLATE must be 0 when HMMR is 1");
        }
        if henableskip {
            return Err("HENABLESKIP must be 0 when HMMR is 1");
        }
    }

    let flags = HalftoneRegionFlags {
        hmmr,
        htemplate,
        henableskip,
        hcombop,
        hdefpixel,
    };

    // 7.4.5.1.2: Halftone grid position and size
    let hgw = reader.read_u32().ok_or("unexpected end of data")?;
    let hgh = reader.read_u32().ok_or("unexpected end of data")?;
    let hgx = reader.read_i32().ok_or("unexpected end of data")?;
    let hgy = reader.read_i32().ok_or("unexpected end of data")?;

    let grid_position_and_size = HalftoneGridPositionAndSize { hgw, hgh, hgx, hgy };

    // 7.4.5.1.3: Halftone grid vector
    let hrx = reader.read_u16().ok_or("unexpected end of data")?;
    let hry = reader.read_u16().ok_or("unexpected end of data")?;

    let grid_vector = HalftoneGridVector { hrx, hry };

    Ok(HalftoneRegionHeader {
        region_info,
        flags,
        grid_position_and_size,
        grid_vector,
    })
}
