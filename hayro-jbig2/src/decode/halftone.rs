//! Halftone region segment parsing and decoding (7.4.5, 6.6).

use alloc::vec;
use alloc::vec::Vec;

use super::pattern::PatternDictionary;
use super::{CombinationOperator, RegionSegmentInfo, Template, parse_region_segment_info};
use crate::bitmap::DecodedRegion;
use crate::error::{DecodeError, ParseError, RegionError, Result};
use crate::gray_scale::{GrayScaleParams, decode_gray_scale_image};
use crate::reader::Reader;

/// Decode a halftone region segment (7.4.5.2, 6.6).
pub(crate) fn decode(
    reader: &mut Reader<'_>,
    pattern_dict: &PatternDictionary,
) -> Result<DecodedRegion> {
    let header = parse(reader)?;
    let region = &header.region_info;

    let mut htreg = DecodedRegion {
        width: region.width,
        height: region.height,
        data: vec![header.flags.initial_pixel_color; (region.width * region.height) as usize],
        x_location: region.x_location,
        y_location: region.y_location,
        combination_operator: region.combination_operator,
    };

    let skip_bitmap = if header.flags.enable_skip {
        Some(compute_skip_bitmap(&header, pattern_dict, &htreg)?)
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
    // TODO: Optimize drawing axis-aligned grids.
    render_patterns(&mut htreg, &gi, &header, pattern_dict)?;

    Ok(htreg)
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

    let grid_width = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let grid_height = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let grid_horizontal_offset = reader.read_i32().ok_or(ParseError::UnexpectedEof)?;
    let grid_vertical_offset = reader.read_i32().ok_or(ParseError::UnexpectedEof)?;

    let grid_position_and_size = HalftoneGridPositionAndSize {
        width: grid_width,
        height: grid_height,
        horizontal_offset: grid_horizontal_offset,
        vertical_offset: grid_vertical_offset,
    };

    let grid_x_vector = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let grid_y_vector = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;

    let grid_vector = HalftoneGridVector {
        x_vector: grid_x_vector,
        y_vector: grid_y_vector,
    };

    Ok(HalftoneRegionHeader {
        region_info,
        flags,
        grid_position_and_size,
        grid_vector,
    })
}

/// Parsed halftone region segment flags (7.4.5.1.1).
#[derive(Debug, Clone)]
struct HalftoneRegionFlags {
    mmr: bool,
    template: Template,
    enable_skip: bool,
    combination_operator: CombinationOperator,
    initial_pixel_color: bool,
}

/// Halftone grid position and size (7.4.5.1.2).
#[derive(Debug, Clone)]
struct HalftoneGridPositionAndSize {
    width: u32,
    height: u32,
    horizontal_offset: i32,
    vertical_offset: i32,
}

/// Halftone grid vector (7.4.5.1.3).
#[derive(Debug, Clone)]
struct HalftoneGridVector {
    /// `HRX` - 256 times the horizontal coordinate of the halftone grid vector.
    x_vector: u16,
    /// `HRY` - 256 times the vertical coordinate of the halftone grid vector.
    y_vector: u16,
}

/// Parsed halftone region segment header (7.4.5.1).
#[derive(Debug, Clone)]
struct HalftoneRegionHeader {
    region_info: RegionSegmentInfo,
    flags: HalftoneRegionFlags,
    grid_position_and_size: HalftoneGridPositionAndSize,
    grid_vector: HalftoneGridVector,
}

/// Compute grid coordinates with checked arithmetic (6.6.5.1, 6.6.5.2).
///
/// Returns (x, y) where:
///   x = (HGX + `m_g` × HRY + `n_g` × HRX) >>_A 8
///   y = (HGY + `m_g` × HRX − `n_g` × HRY) >>_A 8
fn compute_grid_coords(
    grid: &HalftoneGridPositionAndSize,
    vector: &HalftoneGridVector,
    m_g: u32,
    n_g: u32,
) -> Result<(i32, i32)> {
    let hrx = vector.x_vector as i32;
    let hry = vector.y_vector as i32;
    let m_g = m_g as i32;
    let n_g = n_g as i32;

    let x = m_g
        .checked_mul(hry)
        .and_then(|v| v.checked_add(n_g.checked_mul(hrx)?))
        .and_then(|v| grid.horizontal_offset.checked_add(v))
        .ok_or(DecodeError::Overflow)?
        >> 8;

    let y = m_g
        .checked_mul(hrx)
        .and_then(|v| v.checked_sub(n_g.checked_mul(hry)?))
        .and_then(|v| grid.vertical_offset.checked_add(v))
        .ok_or(DecodeError::Overflow)?
        >> 8;

    Ok((x, y))
}

/// Compute the HSKIP bitmap (6.6.5.1).
fn compute_skip_bitmap(
    header: &HalftoneRegionHeader,
    pattern_dict: &PatternDictionary,
    htreg: &DecodedRegion,
) -> Result<Vec<bool>> {
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
            let (x, y) = compute_grid_coords(grid, vector, m_g, n_g)?;

            // "ii) If ((x + HPW ≤ 0) OR (x ≥ HBW) OR (y + HPH ≤ 0) OR (y ≥ HBH))
            // then set: HSKIP[n_g, m_g] = 1" (6.6.5.1)
            let skip = (x + pattern_width <= 0)
                || (x >= region_width)
                || (y + pattern_height <= 0)
                || (y >= region_height);

            hskip[(m_g * grid.width + n_g) as usize] = skip;
        }
    }

    Ok(hskip)
}

/// Render patterns into the target region (6.6.5.2).
fn render_patterns(
    region: &mut DecodedRegion,
    gi: &[u32],
    header: &HalftoneRegionHeader,
    pattern_dict: &PatternDictionary,
) -> Result<()> {
    let grid = &header.grid_position_and_size;
    let vector = &header.grid_vector;

    // "1) For each value of m_g between 0 and HGH − 1, beginning from 0,
    // perform the following steps:" (6.6.5.2)
    for m_g in 0..grid.height {
        // "a) For each value of n_g between 0 and HGW − 1, beginning from 0,
        // perform the following steps:" (6.6.5.2)
        for n_g in 0..grid.width {
            let (x, y) = compute_grid_coords(grid, vector, m_g, n_g)?;

            // "ii) Draw the pattern HPATS[GI[n_g, m_g]] into HTREG such that its
            // upper left pixel is at location (x, y) in HTREG." (6.6.5.2)
            let pattern_index = gi[(m_g * grid.width + n_g) as usize] as usize;

            let pattern = pattern_dict
                .patterns
                .get(pattern_index)
                .ok_or(RegionError::InvalidDimension)?;

            // "Draw pattern at (x, y) using HCOMBOP."
            region.combine(pattern, x, y, header.flags.combination_operator);
        }
    }

    Ok(())
}
