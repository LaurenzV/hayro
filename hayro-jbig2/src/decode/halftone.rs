//! Halftone region segment parsing and decoding (7.4.5, 6.6).

use alloc::vec;
use alloc::vec::Vec;

use super::pattern::PatternDictionary;
use super::{CombinationOperator, RegionSegmentInfo, Template, parse_region_segment_info};
use crate::bitmap::DecodedRegion;
use crate::error::{ParseError, RegionError, Result};
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
    let region = &header.region_info;

    // "1) Fill a bitmap HTREG, of the size given by HBW and HBH, with the
    // HDEFPIXEL value." (6.6.5)
    let mut htreg = DecodedRegion {
        width: region.width,
        height: region.height,
        data: vec![header.flags.initial_pixel_color; (region.width * region.height) as usize],
        x_location: region.x_location,
        y_location: region.y_location,
        combination_operator: region.combination_operator,
    };

    // "2) If HENABLESKIP equals 1, compute a bitmap HSKIP as shown in 6.6.5.1."
    let skip_bitmap = if header.flags.enable_skip {
        Some(compute_hskip(&header, pattern_dict, &htreg))
    } else {
        None
    };

    // "3) Set HBPP to ⌈log₂(HNUMPATS)⌉." (6.6.5)
    let bits_per_pixel = (pattern_dict.patterns.len() as u32)
        .saturating_sub(1)
        .checked_ilog2()
        .map_or(1, |n| n + 1);

    let encoded_data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    // "4) Decode an image GI of size HGW by HGH with HBPP bits per pixel using
    // the gray-scale image decoding procedure as described in Annex C." (6.6.5)
    let gs_params = GrayScaleParams {
        use_mmr: header.flags.mmr,
        bits_per_pixel,
        width: header.grid_position_and_size.width,
        height: header.grid_position_and_size.height,
        template: header.flags.template,
        skip_mask: skip_bitmap.as_deref(),
    };
    let gi = decode_gray_scale_image(encoded_data, &gs_params)?;

    // "5) Place sequentially the patterns corresponding to the values in GI into
    // HTREG by the procedure described in 6.6.5.2." (6.6.5)
    render_patterns(&mut htreg, &gi, &header, pattern_dict)?;

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
    header: &HalftoneRegionHeader,
    pattern_dict: &PatternDictionary,
    htreg: &DecodedRegion,
) -> Vec<bool> {
    let grid = &header.grid_position_and_size;
    let vector = &header.grid_vector;
    let pattern_width = pattern_dict.pattern_width as i32;
    let pattern_height = pattern_dict.pattern_height as i32;
    let region_width = htreg.width as i32;
    let region_height = htreg.height as i32;

    let mut hskip = vec![false; (grid.width * grid.height) as usize];

    // "1) For each value of m_g between 0 and HGH − 1, beginning from 0,
    // perform the following steps:" (6.6.5.1)
    for m_g in 0..grid.height {
        // "a) For each value of n_g between 0 and HGW − 1, beginning from 0,
        // perform the following steps:" (6.6.5.1)
        for n_g in 0..grid.width {
            // "i) Set:
            //    x = (HGX + m_g × HRY + n_g × HRX) >>_A 8
            //    y = (HGY + m_g × HRX − n_g × HRY) >>_A 8" (6.6.5.1)
            let hrx = vector.horizontal_coordinate as i32;
            let hry = vector.vertical_coordinate as i32;
            let x = (grid.horizontal_offset + (m_g as i32) * hry + (n_g as i32) * hrx) >> 8;
            let y = (grid.vertical_offset + (m_g as i32) * hrx - (n_g as i32) * hry) >> 8;

            // "ii) If ((x + HPW ≤ 0) OR (x ≥ HBW) OR (y + HPH ≤ 0) OR (y ≥ HBH))
            // then set: HSKIP[n_g, m_g] = 1" (6.6.5.1)
            let skip = (x + pattern_width <= 0)
                || (x >= region_width)
                || (y + pattern_height <= 0)
                || (y >= region_height);

            hskip[(m_g * grid.width + n_g) as usize] = skip;
        }
    }

    hskip
}

/// Render patterns into HTREG (6.6.5.2).
fn render_patterns(
    htreg: &mut DecodedRegion,
    gi: &[u32],
    header: &HalftoneRegionHeader,
    pattern_dict: &PatternDictionary,
) -> Result<()> {
    let grid = &header.grid_position_and_size;
    let vector = &header.grid_vector;
    let hrx = vector.horizontal_coordinate as i32;
    let hry = vector.vertical_coordinate as i32;

    // "1) For each value of m_g between 0 and HGH − 1, beginning from 0,
    // perform the following steps:" (6.6.5.2)
    for m_g in 0..grid.height {
        // "a) For each value of n_g between 0 and HGW − 1, beginning from 0,
        // perform the following steps:" (6.6.5.2)
        for n_g in 0..grid.width {
            // "i) Set:
            //    x = (HGX + m_g × HRY + n_g × HRX) >>_A 8
            //    y = (HGY + m_g × HRX − n_g × HRY) >>_A 8" (6.6.5.2)
            let x = (grid.horizontal_offset + (m_g as i32) * hry + (n_g as i32) * hrx) >> 8;
            let y = (grid.vertical_offset + (m_g as i32) * hrx - (n_g as i32) * hry) >> 8;

            // "ii) Draw the pattern HPATS[GI[n_g, m_g]] into HTREG such that its
            // upper left pixel is at location (x, y) in HTREG." (6.6.5.2)
            let pattern_index = gi[(m_g * grid.width + n_g) as usize] as usize;

            let pattern = pattern_dict
                .patterns
                .get(pattern_index)
                .ok_or(RegionError::InvalidDimension)?;

            // Draw pattern at (x, y) using HCOMBOP.
            draw_pattern(
                htreg,
                pattern,
                x,
                y,
                pattern_dict,
                header.flags.combination_operator,
            );
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
    pattern_dict: &PatternDictionary,
    combination_operator: CombinationOperator,
) {
    let pattern_width = pattern_dict.pattern_width;
    let pattern_height = pattern_dict.pattern_height;
    let region_width = htreg.width as i32;
    let region_height = htreg.height as i32;

    // "If any part of a decoded pattern, when placed at location (x, y) lies
    // outside the actual halftone-coded bitmap, then this part of the pattern
    // shall be ignored in the process of combining the pattern with the bitmap."
    for py in 0..pattern_height {
        let dest_y = y + py as i32;
        if dest_y < 0 || dest_y >= region_height {
            continue;
        }

        for px in 0..pattern_width {
            let dest_x = x + px as i32;
            if dest_x < 0 || dest_x >= region_width {
                continue;
            }

            let src_pixel = pattern.get_pixel(px, py);
            let dst_pixel = htreg.get_pixel(dest_x as u32, dest_y as u32);

            let result = match combination_operator {
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
