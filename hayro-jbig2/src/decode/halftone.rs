//! Halftone region segment parsing and decoding (7.4.5, 6.6).

use alloc::vec;
use alloc::vec::Vec;

use super::RegionBitmap;
use super::pattern::PatternDictionary;
use super::{CombinationOperator, RegionSegmentInfo, Template, parse_region_segment_info};
use crate::DecodeState;
use crate::bitmap::{Bitmap, MAX_DIMENSION, WORD_BITS};
use crate::cancel::{Cancel, WORD_WORK, WorkCounter};
use crate::error::{OverflowError, ParseError, RegionError, Result, bail};
use crate::gray_scale::{GrayScaleImage, GrayScaleParams, decode_gray_scale_image};
use crate::memory::MemoryBudget;
use crate::reader::Reader;

/// Decode a halftone region segment (7.4.5.2, 6.6).
pub(crate) fn decode(
    header: &HalftoneRegionHeader<'_>,
    pattern_dict: &PatternDictionary,
    ctx: &mut DecodeState<'_>,
) -> Result<RegionBitmap> {
    let region = &header.region_info;

    let mut htreg = Bitmap::new_with(
        region.width,
        region.height,
        region.x_location,
        region.y_location,
        header.flags.initial_pixel_color,
        &mut ctx.budget,
    )?;

    decode_into(header, pattern_dict, &mut htreg, ctx)?;

    Ok(RegionBitmap {
        bitmap: htreg,
        combination_operator: region.combination_operator,
    })
}

pub(crate) fn decode_into(
    header: &HalftoneRegionHeader<'_>,
    pattern_dict: &PatternDictionary,
    htreg: &mut Bitmap,
    ctx: &mut DecodeState<'_>,
) -> Result<()> {
    let skip_bitmap = if header.flags.enable_skip {
        Some(compute_skip_bitmap(
            header,
            pattern_dict,
            htreg,
            &mut ctx.budget,
            ctx.cancel,
        )?)
    } else {
        None
    };

    // "3) Set HBPP to ⌈log₂(HNUMPATS)⌉." (6.6.5)
    let bits_per_pixel = (pattern_dict.patterns.len() as u32)
        .saturating_sub(1)
        .checked_ilog2()
        .map_or(1, |n| n + 1);

    // "4) Decode an image GI of size HGW by HGH with HBPP bits per pixel using
    // the gray-scale image decoding procedure as described in Annex C." (6.6.5)
    let gs_params = GrayScaleParams {
        use_mmr: header.flags.mmr,
        bits_per_pixel,
        width: header.grid_position_and_size.width,
        height: header.grid_position_and_size.height,
        template: header.flags.template,
        skip_mask: skip_bitmap.as_deref(),
        strict: header.strict,
    };
    let gi = decode_gray_scale_image(header.data, &gs_params, ctx)?;

    // "5) Place sequentially the patterns corresponding to the values in GI into
    // HTREG by the procedure described in 6.6.5.2." (6.6.5)
    render_patterns(
        htreg,
        &gi,
        header,
        pattern_dict,
        &mut ctx.budget,
        ctx.cancel,
    )?;

    Ok(())
}

/// Parse a halftone region segment header (7.4.5.1).
pub(crate) fn parse<'a>(reader: &mut Reader<'a>, strict: bool) -> Result<HalftoneRegionHeader<'a>> {
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

    if grid_width > MAX_DIMENSION || grid_height > MAX_DIMENSION {
        bail!(OverflowError::GridDimension);
    }

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

    let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    Ok(HalftoneRegionHeader {
        region_info,
        flags,
        grid_position_and_size,
        grid_vector,
        data,
        strict,
    })
}

/// Parsed halftone region segment flags (7.4.5.1.1).
#[derive(Debug, Clone)]
pub(crate) struct HalftoneRegionFlags {
    pub(crate) mmr: bool,
    pub(crate) template: Template,
    pub(crate) enable_skip: bool,
    pub(crate) combination_operator: CombinationOperator,
    pub(crate) initial_pixel_color: bool,
}

/// Halftone grid position and size (7.4.5.1.2).
#[derive(Debug, Clone)]
pub(crate) struct HalftoneGridPositionAndSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) horizontal_offset: i32,
    pub(crate) vertical_offset: i32,
}

/// Halftone grid vector (7.4.5.1.3).
#[derive(Debug, Clone)]
pub(crate) struct HalftoneGridVector {
    /// `HRX` - 256 times the horizontal coordinate of the halftone grid vector.
    pub(crate) x_vector: u16,
    /// `HRY` - 256 times the vertical coordinate of the halftone grid vector.
    pub(crate) y_vector: u16,
}

/// Parsed halftone region segment header (7.4.5.1).
#[derive(Debug, Clone)]
pub(crate) struct HalftoneRegionHeader<'a> {
    pub(crate) region_info: RegionSegmentInfo,
    pub(crate) flags: HalftoneRegionFlags,
    pub(crate) grid_position_and_size: HalftoneGridPositionAndSize,
    pub(crate) grid_vector: HalftoneGridVector,
    pub(crate) data: &'a [u8],
    pub(crate) strict: bool,
}

struct GridCoords {
    x: i64,
    y: i64,
    row_x: i64,
    row_y: i64,
    hrx: i64,
    hry: i64,
}

impl GridCoords {
    fn new(grid: &HalftoneGridPositionAndSize, vector: &HalftoneGridVector) -> Self {
        Self {
            x: grid.horizontal_offset as i64,
            y: grid.vertical_offset as i64,
            row_x: grid.horizontal_offset as i64,
            row_y: grid.vertical_offset as i64,
            hrx: vector.x_vector as i64,
            hry: vector.y_vector as i64,
        }
    }

    #[inline]
    fn get(&self) -> (i64, i64) {
        (self.x >> 8, self.y >> 8)
    }

    #[inline]
    fn advance_col(&mut self) {
        self.x += self.hrx;
        self.y -= self.hry;
    }

    #[inline]
    fn advance_row(&mut self) {
        self.row_x += self.hry;
        self.row_y += self.hrx;
        self.x = self.row_x;
        self.y = self.row_y;
    }
}

/// Compute the HSKIP bitmap (6.6.5.1).
fn compute_skip_bitmap(
    header: &HalftoneRegionHeader<'_>,
    pattern_dict: &PatternDictionary,
    htreg: &Bitmap,
    budget: &mut MemoryBudget,
    cancel: Cancel<'_>,
) -> Result<Vec<u32>> {
    let grid = &header.grid_position_and_size;

    let stride = grid.width.div_ceil(32);
    let len = stride
        .checked_mul(grid.height)
        .ok_or(OverflowError::BitmapDimension)? as usize;
    budget.charge(len * size_of::<u32>())?;
    let mut hskip = vec![0_u32; len];
    let mut coords = GridCoords::new(grid, &header.grid_vector);
    let mut work = WorkCounter::new();

    for m_g in 0..grid.height {
        for n_g in 0..grid.width {
            work.add(1, cancel)?;
            let (x, y) = coords.get();

            if lies_outside(x, y, pattern_dict, htreg) {
                let word_idx = (m_g * stride + n_g / 32) as usize;
                let bit_pos = 31 - (n_g % 32);
                hskip[word_idx] |= 1 << bit_pos;
            }

            coords.advance_col();
        }
        coords.advance_row();
    }

    Ok(hskip)
}

fn lies_outside(x: i64, y: i64, pattern_dict: &PatternDictionary, region: &Bitmap) -> bool {
    x + i64::from(pattern_dict.pattern_width) <= 0
        || x >= i64::from(region.width)
        || y + i64::from(pattern_dict.pattern_height) <= 0
        || y >= i64::from(region.height)
}

/// Render patterns into the target region (6.6.5.2).
fn render_patterns(
    region: &mut Bitmap,
    gi: &GrayScaleImage,
    header: &HalftoneRegionHeader<'_>,
    pattern_dict: &PatternDictionary,
    budget: &mut MemoryBudget,
    cancel: Cancel<'_>,
) -> Result<()> {
    let grid = &header.grid_position_and_size;
    let op = header.flags.combination_operator;
    let pw = pattern_dict.pattern_width;
    let ph = pattern_dict.pattern_height;
    let stride = region.stride;
    let num_patterns = pattern_dict.patterns.len();

    // The original pattern mask has 1s in the top `pw` bits.
    let original_mask = if pw >= WORD_BITS {
        !0_u32
    } else {
        !0_u32 << (WORD_BITS - pw)
    };

    macro_rules! blit_rows {
        ($data:expr, $pattern:expr, $offset:expr, $yu:expr, $first_word:expr, $two:expr,
         |$d0:ident, $s0:ident| $op0:expr, |$d1:ident, $s1:ident| $op1:expr) => {
            if $two {
                for row in 0..ph {
                    let base = (($yu + row) * stride + $first_word) as usize;
                    let word = $pattern[row as usize];
                    let $d0 = $data[base];
                    let $s0 = word >> $offset;
                    $data[base] = $op0;
                    let $d1 = $data[base + 1];
                    let $s1 = word << (WORD_BITS - $offset);
                    $data[base + 1] = $op1;
                }
            } else {
                for row in 0..ph {
                    let base = (($yu + row) * stride + $first_word) as usize;
                    let $d0 = $data[base];
                    let $s0 = $pattern[row as usize] >> $offset;
                    $data[base] = $op0;
                }
            }
        };
    }

    let mut coords = GridCoords::new(grid, &header.grid_vector);
    budget.charge(grid.width as usize * size_of::<u32>())?;
    let mut gray_values = vec![0; grid.width as usize];
    let mut work = WorkCounter::new();
    let cell_work = 1 + u64::from(WORD_WORK) * u64::from(ph) * u64::from(pw.div_ceil(WORD_BITS));

    for m_g in 0..grid.height {
        gi.row_values(m_g, &mut gray_values);

        for &gray_value in &gray_values {
            work.add(cell_work, cancel)?;
            let (x, y) = coords.get();
            let gray_value = gray_value as usize;

            if header.strict && gray_value >= num_patterns {
                bail!(RegionError::GrayScaleOutOfRange);
            }

            let pattern_index = gray_value.min(num_patterns - 1);

            let pattern = &pattern_dict.patterns[pattern_index];

            if lies_outside(x, y, pattern_dict, region) {
                coords.advance_col();
                continue;
            }

            // Patterns that are at most one word wide and lie inside the region
            // can be blitted row by row.
            if pw <= WORD_BITS
                && x >= 0
                && y >= 0
                && x + i64::from(pw) <= i64::from(region.width)
                && y + i64::from(ph) <= i64::from(region.height)
            {
                let pattern = &pattern.data;
                let xu = x as u32;
                let yu = y as u32;
                let first_word = xu / WORD_BITS;
                let offset = xu % WORD_BITS;
                let two = offset != 0 && first_word + 1 < stride;

                match op {
                    CombinationOperator::Or => {
                        blit_rows!(
                            region.data,
                            pattern,
                            offset,
                            yu,
                            first_word,
                            two,
                            |d, s| d | s,
                            |d, s| d | s
                        );
                    }
                    CombinationOperator::Xor => {
                        blit_rows!(
                            region.data,
                            pattern,
                            offset,
                            yu,
                            first_word,
                            two,
                            |d, s| d ^ s,
                            |d, s| d ^ s
                        );
                    }
                    CombinationOperator::And => {
                        let inv_mask0 = !(original_mask >> offset);
                        let inv_mask1 = if offset == 0 {
                            !0
                        } else {
                            !(original_mask << (WORD_BITS - offset))
                        };
                        blit_rows!(
                            region.data,
                            pattern,
                            offset,
                            yu,
                            first_word,
                            two,
                            |d, s| d & (s | inv_mask0),
                            |d, s| d & (s | inv_mask1)
                        );
                    }
                    CombinationOperator::Replace => {
                        let inv_mask0 = !(original_mask >> offset);
                        let inv_mask1 = if offset == 0 {
                            !0
                        } else {
                            !(original_mask << (WORD_BITS - offset))
                        };
                        blit_rows!(
                            region.data,
                            pattern,
                            offset,
                            yu,
                            first_word,
                            two,
                            |d, s| (d & inv_mask0) | s,
                            |d, s| (d & inv_mask1) | s
                        );
                    }
                    CombinationOperator::Xnor => {
                        let mask0 = original_mask >> offset;
                        let mask1 = if offset == 0 {
                            0
                        } else {
                            original_mask << (WORD_BITS - offset)
                        };
                        let inv_mask0 = !mask0;
                        let inv_mask1 = !mask1;
                        blit_rows!(
                            region.data,
                            pattern,
                            offset,
                            yu,
                            first_word,
                            two,
                            |d, s| (d & inv_mask0) | (!(d ^ s) & mask0),
                            |d, s| (d & inv_mask1) | (!(d ^ s) & mask1)
                        );
                    }
                }
            } else {
                region.combine(pattern, x as i32, y as i32, op);
            }

            coords.advance_col();
        }
        coords.advance_row();
    }

    Ok(())
}
