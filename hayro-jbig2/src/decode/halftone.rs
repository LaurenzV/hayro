//! Halftone region segment parsing and decoding (7.4.5, 6.6).

use alloc::vec;
use alloc::vec::Vec;

use super::pattern::PatternDictionary;
use super::{CombinationOperator, RegionSegmentInfo, Template, parse_region_segment_info};
use crate::bitmap::DecodedRegion;
use crate::error::{ParseError, RegionError, Result, TemplateError, bail};
use crate::gray_scale::{GrayScaleParams, decode_gray_scale_image};
use crate::reader::Reader;

/// Decode a halftone region segment (7.4.5.2, 6.6).
///
/// "A halftone region segment is decoded according to the following steps:
/// 1) Interpret its header, as described in 7.4.5.1.
/// 2) Decode (or retrieve the results of decoding) the referred-to pattern
///    dictionary segment.
/// 3) As described in E.3.7, reset all the arithmetic coding statistics to zero.
/// 4) Invoke the halftone region decoding procedure described in 6.6."
pub(crate) fn decode(
    reader: &mut Reader<'_>,
    pattern_dict: &PatternDictionary,
) -> Result<DecodedRegion> {
    let header = parse(reader)?;

    let hbw = header.region_info.width;
    let hbh = header.region_info.height;
    let hgw = header.grid_position_and_size.width;
    let hgh = header.grid_position_and_size.height;
    let hgx = header.grid_position_and_size.horizontal_offset;
    let hgy = header.grid_position_and_size.vertical_offset;
    let hrx = header.grid_vector.horizontal_coordinate as i32;
    let hry = header.grid_vector.vertical_coordinate as i32;
    let hpw = pattern_dict.pattern_width;
    let hph = pattern_dict.pattern_height;
    let hnumpats = pattern_dict.patterns.len() as u32;

    // "1) Fill a bitmap HTREG, of the size given by HBW and HBH, with the
    // HDEFPIXEL value." (6.6.5)
    let mut htreg = DecodedRegion {
        width: hbw,
        height: hbh,
        data: vec![header.flags.initial_pixel_color; (hbw * hbh) as usize],
        x_location: header.region_info.x_location,
        y_location: header.region_info.y_location,
        combination_operator: header.region_info.combination_operator,
    };

    // "2) If HENABLESKIP equals 1, compute a bitmap HSKIP as shown in 6.6.5.1."
    let hskip = if header.flags.enable_skip {
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
        use_mmr: header.flags.mmr,
        bits_per_pixel: hbpp,
        width: hgw,
        height: hgh,
        template: header.flags.template,
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
        header.flags.combination_operator,
    )?;

    Ok(htreg)
}

/// Parsed halftone region segment flags (7.4.5.1.1).
#[derive(Debug, Clone)]
struct HalftoneRegionFlags {
    mmr: bool,
    template: Template,
    /// `HENABLESKIP`
    enable_skip: bool,
    /// `HCOMBOP`
    combination_operator: CombinationOperator,
    /// `HDEFPIXEL`
    initial_pixel_color: bool,
}

/// Halftone grid position and size (7.4.5.1.2).
#[derive(Debug, Clone)]
struct HalftoneGridPositionAndSize {
    /// `HGW`
    width: u32,
    /// `HGH`
    height: u32,
    /// `HGX`
    horizontal_offset: i32,
    /// `HGY`
    vertical_offset: i32,
}

/// Halftone grid vector (7.4.5.1.3).
#[derive(Debug, Clone)]
struct HalftoneGridVector {
    /// "`HRX`: This unsigned two-byte field contains 256 times the horizontal
    /// coordinate of the halftone grid vector."
    horizontal_coordinate: u16,
    /// "`HRY`: This unsigned two-byte field contains 256 times the vertical
    /// coordinate of the halftone grid vector."
    vertical_coordinate: u16,
}

/// Parsed halftone region segment header (7.4.5.1).
#[derive(Debug, Clone)]
struct HalftoneRegionHeader {
    region_info: RegionSegmentInfo,
    flags: HalftoneRegionFlags,
    grid_position_and_size: HalftoneGridPositionAndSize,
    grid_vector: HalftoneGridVector,
}

/// Parse a halftone region segment header (7.4.5.1).
fn parse(reader: &mut Reader<'_>) -> Result<HalftoneRegionHeader> {
    let region_info = parse_region_segment_info(reader)?;
    let flags_byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let mmr = flags_byte & 0x01 != 0;
    let template = Template::from_byte(flags_byte >> 1);
    let enable_skip = flags_byte & 0x08 != 0;
    let combination_operator = CombinationOperator::from_value(flags_byte >> 4)?;
    let initial_pixel_color = flags_byte & 0x80 != 0;

    let flags = HalftoneRegionFlags {
        mmr,
        template,
        enable_skip,
        combination_operator,
        initial_pixel_color,
    };

    // 7.4.5.1.2: Halftone grid position and size
    let hgw = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let hgh = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let hgx = reader.read_i32().ok_or(ParseError::UnexpectedEof)?;
    let hgy = reader.read_i32().ok_or(ParseError::UnexpectedEof)?;

    let grid_position_and_size = HalftoneGridPositionAndSize {
        width: hgw,
        height: hgh,
        horizontal_offset: hgx,
        vertical_offset: hgy,
    };

    // 7.4.5.1.3: Halftone grid vector
    let hrx = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let hry = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;

    let grid_vector = HalftoneGridVector {
        horizontal_coordinate: hrx,
        vertical_coordinate: hry,
    };

    Ok(HalftoneRegionHeader {
        region_info,
        flags,
        grid_position_and_size,
        grid_vector,
    })
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
