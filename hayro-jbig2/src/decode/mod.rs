pub(crate) mod generic;
pub(crate) mod generic_refinement;
pub(crate) mod halftone;
pub(crate) mod pattern;
pub(crate) mod symbol;
pub(crate) mod text;

use crate::bitmap::Bitmap;
use crate::decode::RefinementTemplate::{Template0, Template1};
use crate::error::{DecodeError, ParseError, RegionError, Result, bail, err};
use crate::reader::Reader;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub(crate) struct RegionBitmap {
    pub(crate) bitmap: Bitmap,
    pub(crate) combination_operator: CombinationOperator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CombinationOperator {
    #[default]
    Or,
    And,
    Xor,
    Xnor,
    Replace,
}

impl CombinationOperator {
    pub(crate) fn from_value(value: u8) -> Result<Self> {
        match value & 0x07 {
            0 => Ok(Self::Or),
            1 => Ok(Self::And),
            2 => Ok(Self::Xor),
            3 => Ok(Self::Xnor),
            4 => Ok(Self::Replace),
            _ => err!(RegionError::InvalidCombinationOperator),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Template {
    Template0 = 0,
    Template1 = 1,
    Template2 = 2,
    Template3 = 3,
}

impl Template {
    pub(crate) fn from_byte(value: u8) -> Self {
        match value & 0x03 {
            0 => Self::Template0,
            1 => Self::Template1,
            2 => Self::Template2,
            3 => Self::Template3,
            _ => unreachable!(),
        }
    }

    pub(crate) fn context_bits(self) -> usize {
        match self {
            Self::Template0 => 16,
            Self::Template1 => 13,
            Self::Template2 | Self::Template3 => 10,
        }
    }

    pub(crate) fn adaptive_template_pixels(&self) -> u8 {
        match self {
            Self::Template0 => 4,
            Self::Template1 | Self::Template2 | Self::Template3 => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RegionSegmentInfo {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) x_location: u32,
    pub(crate) y_location: u32,
    pub(crate) combination_operator: CombinationOperator,
}

pub(crate) fn parse_region_segment_info(reader: &mut Reader<'_>) -> Result<RegionSegmentInfo> {
    let width = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let height = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let x_location = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let y_location = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    let combination_operator = CombinationOperator::from_value(flags)?;

    if flags & 0x08 != 0 {
        bail!(DecodeError::Unsupported);
    }

    Ok(RegionSegmentInfo {
        width,
        height,
        x_location,
        y_location,
        combination_operator,
    })
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AdaptiveTemplatePixel {
    pub(crate) x: i8,
    pub(crate) y: i8,
}

pub(crate) fn parse_refinement_at_pixels(
    reader: &mut Reader<'_>,
) -> Result<Vec<AdaptiveTemplatePixel>> {
    let x1 = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;
    let y1 = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;

    let x2 = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;
    let y2 = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;

    Ok(vec![
        AdaptiveTemplatePixel { x: x1, y: y1 },
        AdaptiveTemplatePixel { x: x2, y: y2 },
    ])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefinementTemplate {
    Template0 = 0,
    Template1 = 1,
}

impl RefinementTemplate {
    pub(crate) fn from_byte(value: u8) -> Self {
        if value & 0x01 == 0 {
            Template0
        } else {
            Template1
        }
    }

    pub(crate) fn context_bits(&self) -> usize {
        match self {
            Self::Template0 => 13,
            Self::Template1 => 10,
        }
    }
}
