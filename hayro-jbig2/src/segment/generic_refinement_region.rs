//! Generic refinement region segment parsing (7.4.7).

use crate::reader::Reader;
use crate::segment::region::{RegionSegmentInfo, parse_region_segment_info};

/// Adaptive template pixel position for refinement regions.
///
/// "The AT coordinate X and Y fields are signed values, and may take on values
/// that are permitted according to 6.3.5.3." (7.4.7.3)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RefinementAdaptiveTemplatePixel {
    pub x: i8,
    pub y: i8,
}

/// Template used for refinement arithmetic coding (7.4.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrTemplate {
    /// Template 0: 13 pixels (6.3.5.2, Figure 12)
    Template0 = 0,
    /// Template 1: 10 pixels (6.3.5.2, Figure 13)
    Template1 = 1,
}

/// Parsed generic refinement region segment header (7.4.7.1).
#[derive(Debug, Clone)]
pub(crate) struct GenericRefinementRegionHeader {
    /// Region segment information field (7.4.1).
    pub region_info: RegionSegmentInfo,
    /// "Bit 0: GRTEMPLATE. This field specifies the template used for
    /// template-based arithmetic coding." (7.4.7.2)
    pub gr_template: GrTemplate,
    /// "Bit 1: TPGRON. This field specifies whether typical prediction for
    /// generic refinement is used." (7.4.7.2)
    pub tpgron: bool,
    /// Adaptive template pixels (7.4.7.3).
    ///
    /// "This field is only present if GRTEMPLATE is 0."
    /// Contains 2 AT pixels (4 bytes): GRATX1, GRATY1, GRATX2, GRATY2
    pub adaptive_template_pixels: Vec<RefinementAdaptiveTemplatePixel>,
}

/// Parse a generic refinement region segment header (7.4.7.1).
pub(crate) fn parse_generic_refinement_region_header(
    reader: &mut Reader<'_>,
) -> Result<GenericRefinementRegionHeader, &'static str> {
    // 7.4.7.1: "The data part of a generic refinement region segment begins
    // with a generic refinement region segment data header. This header
    // contains the fields shown in Figure 52."

    // Region segment information field (7.4.1)
    let region_info = parse_region_segment_info(reader)?;

    // 7.4.7.2: Generic refinement region segment flags
    // "This one-byte field is formatted as shown in Figure 53."
    let flags = reader.read_byte().ok_or("unexpected end of data")?;

    // "Bit 0: GRTEMPLATE"
    let gr_template = if flags & 0x01 == 0 {
        GrTemplate::Template0
    } else {
        GrTemplate::Template1
    };

    // "Bit 1: TPGRON"
    let tpgron = flags & 0x02 != 0;

    // 7.4.7.3: Generic refinement region segment AT flags
    // "This field is only present if GRTEMPLATE is 0."
    let adaptive_template_pixels = if gr_template == GrTemplate::Template0 {
        parse_refinement_adaptive_template_pixels(reader)?
    } else {
        Vec::new()
    };

    Ok(GenericRefinementRegionHeader {
        region_info,
        gr_template,
        tpgron,
        adaptive_template_pixels,
    })
}

/// Parse refinement adaptive template pixel positions (7.4.7.3).
///
/// "It is a four-byte field, formatted as shown in Figure 54."
fn parse_refinement_adaptive_template_pixels(
    reader: &mut Reader<'_>,
) -> Result<Vec<RefinementAdaptiveTemplatePixel>, &'static str> {
    let mut pixels = Vec::with_capacity(2);

    // GRATX1, GRATY1
    let x1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x1, y: y1 });

    // GRATX2, GRATY2
    let x2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x2, y: y2 });

    Ok(pixels)
}
