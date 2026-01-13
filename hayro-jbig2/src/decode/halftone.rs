//! Halftone region segment parsing and decoding (7.4.5, 6.6).

use alloc::vec;
use alloc::vec::Vec;

use super::pattern::PatternDictionary;
use super::{CombinationOperator, RegionSegmentInfo, Template, parse_region_segment_info};
use crate::bitmap::DecodedRegion;
use crate::error::{ParseError, RegionError, Result, TemplateError, bail};
use crate::gray_scale::{GrayScaleParams, decode_gray_scale_image};
use crate::reader::Reader;

/// Parsed halftone region segment flags (7.4.5.1.1).
///
/// "This one-byte field is formatted as shown in Figure 44."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneRegionFlags {
    /// "Bit 0: HMMR. If this bit is 1, then the segment uses the MMR encoding
    /// variant. If this bit is 0, then the segment uses the arithmetic encoding
    /// variant."
    pub(crate) hmmr: bool,
    /// "Bits 1-2: HTEMPLATE. This field controls the template used to decode
    /// halftone gray-scale value bitplanes if HMMR is 0. If HMMR is 1, this
    /// field must contain the value 0."
    pub(crate) htemplate: Template,
    /// "Bit 3: HENABLESKIP. This field controls whether gray-scale values that
    /// do not contribute to the region contents are skipped during decoding.
    /// If HMMR is 1, this field must contain the value 0."
    pub(crate) henableskip: bool,
    /// "Bits 4-6: HCOMBOP. This field has five possible values, representing
    /// one of five possible combination operators."
    pub(crate) hcombop: CombinationOperator,
    /// "Bit 7: HDEFPIXEL. This bit contains the initial value for every pixel
    /// in the halftone region, before any patterns are drawn."
    pub(crate) hdefpixel: bool,
}

/// Halftone grid position and size (7.4.5.1.2).
///
/// "This field describes the location and size of the grid of gray-scale values."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneGridPositionAndSize {
    /// "HGW: This four-byte field contains the width of the array of gray-scale
    /// values." (7.4.5.1.2.1)
    pub(crate) hgw: u32,
    /// "HGH: This four-byte field contains the height of the array of gray-scale
    /// values." (7.4.5.1.2.2)
    pub(crate) hgh: u32,
    /// "HGX: This signed four-byte field contains 256 times the horizontal offset
    /// of the origin of the halftone grid." (7.4.5.1.2.3)
    pub(crate) hgx: i32,
    /// "HGY: This signed four-byte field contains 256 times the vertical offset
    /// of the origin of the halftone grid." (7.4.5.1.2.4)
    pub(crate) hgy: i32,
}

/// Halftone grid vector (7.4.5.1.3).
///
/// "This field describes the vector used to draw the grid of gray-scale values."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneGridVector {
    /// "HRX: This unsigned two-byte field contains 256 times the horizontal
    /// coordinate of the halftone grid vector." (7.4.5.1.3.1)
    pub(crate) hrx: u16,
    /// "HRY: This unsigned two-byte field contains 256 times the vertical
    /// coordinate of the halftone grid vector." (7.4.5.1.3.2)
    pub(crate) hry: u16,
}

/// Parsed halftone region segment header (7.4.5.1).
///
/// "The data part of a halftone region segment begins with a halftone region
/// segment data header. This header contains the fields shown in Figure 43."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneRegionHeader {
    /// Region segment information field (7.4.1).
    pub(crate) region_info: RegionSegmentInfo,
    /// Halftone region segment flags (7.4.5.1.1).
    pub(crate) flags: HalftoneRegionFlags,
    /// Halftone grid position and size (7.4.5.1.2).
    pub(crate) grid_position_and_size: HalftoneGridPositionAndSize,
    /// Halftone grid vector (7.4.5.1.3).
    pub(crate) grid_vector: HalftoneGridVector,
}

/// Parse a halftone region segment header (7.4.5.1).
pub(crate) fn parse_halftone_region_header(
    reader: &mut Reader<'_>,
) -> Result<HalftoneRegionHeader> {
    // Region segment information field (7.4.1)
    let region_info = parse_region_segment_info(reader)?;

    // 7.4.5.1.1: Halftone region segment flags
    let flags_byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    // "Bit 0: HMMR"
    let hmmr = flags_byte & 0x01 != 0;

    // "Bits 1-2: HTEMPLATE"
    let htemplate = Template::from_byte(flags_byte >> 1);

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
        _ => bail!(RegionError::InvalidCombinationOperator),
    };

    // "Bit 7: HDEFPIXEL"
    let hdefpixel = flags_byte & 0x80 != 0;

    // Validate constraints when HMMR is 1
    if hmmr {
        if htemplate != Template::Template0 {
            bail!(TemplateError::Invalid);
        }
        if henableskip {
            bail!(TemplateError::Invalid);
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
    let hgw = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let hgh = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let hgx = reader.read_i32().ok_or(ParseError::UnexpectedEof)?;
    let hgy = reader.read_i32().ok_or(ParseError::UnexpectedEof)?;

    let grid_position_and_size = HalftoneGridPositionAndSize { hgw, hgh, hgx, hgy };

    // 7.4.5.1.3: Halftone grid vector
    let hrx = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let hry = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;

    let grid_vector = HalftoneGridVector { hrx, hry };

    Ok(HalftoneRegionHeader {
        region_info,
        flags,
        grid_position_and_size,
        grid_vector,
    })
}

/// Decode a halftone region segment (7.4.5.2, 6.6).
///
/// "A halftone region segment is decoded according to the following steps:
/// 1) Interpret its header, as described in 7.4.5.1.
/// 2) Decode (or retrieve the results of decoding) the referred-to pattern
///    dictionary segment.
/// 3) As described in E.3.7, reset all the arithmetic coding statistics to zero.
/// 4) Invoke the halftone region decoding procedure described in 6.6."
pub(crate) fn decode_halftone_region(
    reader: &mut Reader<'_>,
    pattern_dict: &PatternDictionary,
) -> Result<DecodedRegion> {
    let header = parse_halftone_region_header(reader)?;

    let hbw = header.region_info.width;
    let hbh = header.region_info.height;
    let hgw = header.grid_position_and_size.hgw;
    let hgh = header.grid_position_and_size.hgh;
    let hgx = header.grid_position_and_size.hgx;
    let hgy = header.grid_position_and_size.hgy;
    let hrx = header.grid_vector.hrx as i32;
    let hry = header.grid_vector.hry as i32;
    let hpw = pattern_dict.pattern_width;
    let hph = pattern_dict.pattern_height;
    let hnumpats = pattern_dict.patterns.len() as u32;

    // "1) Fill a bitmap HTREG, of the size given by HBW and HBH, with the
    // HDEFPIXEL value." (6.6.5)
    let mut htreg = DecodedRegion {
        width: hbw,
        height: hbh,
        data: vec![header.flags.hdefpixel; (hbw * hbh) as usize],
        x_location: header.region_info.x_location,
        y_location: header.region_info.y_location,
        combination_operator: header.region_info.combination_operator,
    };

    // "2) If HENABLESKIP equals 1, compute a bitmap HSKIP as shown in 6.6.5.1."
    let hskip = if header.flags.henableskip {
        Some(compute_hskip(
            hgw, hgh, hgx, hgy, hrx, hry, hpw, hph, hbw, hbh,
        ))
    } else {
        None
    };

    // "3) Set HBPP to ⌈log₂(HNUMPATS)⌉." (6.6.5)
    let hbpp = hnumpats
        .saturating_sub(1)
        .checked_ilog2()
        .map_or(1, |n| n + 1);

    let encoded_data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    // "4) Decode an image GI of size HGW by HGH with HBPP bits per pixel using
    // the gray-scale image decoding procedure as described in Annex C." (6.6.5)
    //
    // "The parameters to this decoding procedure are shown in Table 23." (6.6.5)
    let gs_params = GrayScaleParams {
        use_mmr: header.flags.hmmr,
        bits_per_pixel: hbpp,
        width: hgw,
        height: hgh,
        template: header.flags.htemplate,
        skip_mask: hskip.as_deref(),
    };
    let gi = decode_gray_scale_image(encoded_data, &gs_params)?;

    // "5) Place sequentially the patterns corresponding to the values in GI into
    // HTREG by the procedure described in 6.6.5.2." (6.6.5)
    render_patterns(
        &mut htreg,
        &gi,
        hgw,
        hgh,
        hgx,
        hgy,
        hrx,
        hry,
        pattern_dict,
        header.flags.hcombop,
    )?;

    Ok(htreg)
}

/// Compute the HSKIP bitmap (6.6.5.1).
///
/// "The bitmap HSKIP contains 1 at a pixel if drawing a pattern at the
/// corresponding location on the halftone grid does not affect any pixels
/// of HTREG."
fn compute_hskip(
    hgw: u32,
    hgh: u32,
    hgx: i32,
    hgy: i32,
    hrx: i32,
    hry: i32,
    hpw: u32,
    hph: u32,
    hbw: u32,
    hbh: u32,
) -> Vec<bool> {
    let mut hskip = vec![false; (hgw * hgh) as usize];

    // "1) For each value of m_g between 0 and HGH − 1, beginning from 0,
    // perform the following steps:" (6.6.5.1)
    for m_g in 0..hgh {
        // "a) For each value of n_g between 0 and HGW − 1, beginning from 0,
        // perform the following steps:" (6.6.5.1)
        for n_g in 0..hgw {
            // "i) Set:
            //    x = (HGX + m_g × HRY + n_g × HRX) >>_A 8
            //    y = (HGY + m_g × HRX − n_g × HRY) >>_A 8" (6.6.5.1)
            let x = (hgx + (m_g as i32) * hry + (n_g as i32) * hrx) >> 8;
            let y = (hgy + (m_g as i32) * hrx - (n_g as i32) * hry) >> 8;

            // "ii) If ((x + HPW ≤ 0) OR (x ≥ HBW) OR (y + HPH ≤ 0) OR (y ≥ HBH))
            // then set: HSKIP[n_g, m_g] = 1" (6.6.5.1)
            let skip = (x + hpw as i32 <= 0)
                || (x >= hbw as i32)
                || (y + hph as i32 <= 0)
                || (y >= hbh as i32);

            hskip[(m_g * hgw + n_g) as usize] = skip;
        }
    }

    hskip
}

/// Render patterns into HTREG (6.6.5.2).
fn render_patterns(
    htreg: &mut DecodedRegion,
    gi: &[u32],
    hgw: u32,
    hgh: u32,
    hgx: i32,
    hgy: i32,
    hrx: i32,
    hry: i32,
    pattern_dict: &PatternDictionary,
    hcombop: CombinationOperator,
) -> Result<()> {
    let hpw = pattern_dict.pattern_width;
    let hph = pattern_dict.pattern_height;
    let hbw = htreg.width;
    let hbh = htreg.height;

    // "1) For each value of m_g between 0 and HGH − 1, beginning from 0,
    // perform the following steps:" (6.6.5.2)
    for m_g in 0..hgh {
        // "a) For each value of n_g between 0 and HGW − 1, beginning from 0,
        // perform the following steps:" (6.6.5.2)
        for n_g in 0..hgw {
            // "i) Set:
            //    x = (HGX + m_g × HRY + n_g × HRX) >>_A 8
            //    y = (HGY + m_g × HRX − n_g × HRY) >>_A 8" (6.6.5.2)
            let x = (hgx + (m_g as i32) * hry + (n_g as i32) * hrx) >> 8;
            let y = (hgy + (m_g as i32) * hrx - (n_g as i32) * hry) >> 8;

            // "ii) Draw the pattern HPATS[GI[n_g, m_g]] into HTREG such that its
            // upper left pixel is at location (x, y) in HTREG." (6.6.5.2)
            let pattern_index = gi[(m_g * hgw + n_g) as usize] as usize;

            let pattern = pattern_dict
                .patterns
                .get(pattern_index)
                .ok_or(RegionError::InvalidDimension)?;

            // Draw pattern at (x, y) using HCOMBOP.
            draw_pattern(htreg, pattern, x, y, hpw, hph, hbw, hbh, hcombop);
        }
    }

    Ok(())
}

/// Draw a pattern into the halftone region at the specified location.
///
/// "A pattern is drawn into HTREG as follows. Each pixel of the pattern shall
/// be combined with the current value of the corresponding pixel in the
/// halftone-coded bitmap, using the combination operator specified by HCOMBOP."
fn draw_pattern(
    htreg: &mut DecodedRegion,
    pattern: &DecodedRegion,
    x: i32,
    y: i32,
    hpw: u32,
    hph: u32,
    hbw: u32,
    hbh: u32,
    hcombop: CombinationOperator,
) {
    // "If any part of a decoded pattern, when placed at location (x, y) lies
    // outside the actual halftone-coded bitmap, then this part of the pattern
    // shall be ignored in the process of combining the pattern with the bitmap."
    for py in 0..hph {
        let dest_y = y + py as i32;
        if dest_y < 0 || dest_y >= hbh as i32 {
            continue;
        }

        for px in 0..hpw {
            let dest_x = x + px as i32;
            if dest_x < 0 || dest_x >= hbw as i32 {
                continue;
            }

            let src_pixel = pattern.get_pixel(px, py);
            let dst_pixel = htreg.get_pixel(dest_x as u32, dest_y as u32);

            let result = match hcombop {
                CombinationOperator::Or => dst_pixel | src_pixel,
                CombinationOperator::And => dst_pixel & src_pixel,
                CombinationOperator::Xor => dst_pixel ^ src_pixel,
                CombinationOperator::Xnor => !(dst_pixel ^ src_pixel),
                CombinationOperator::Replace => src_pixel,
            };

            htreg.set_pixel(dest_x as u32, dest_y as u32, result);
        }
    }
}
