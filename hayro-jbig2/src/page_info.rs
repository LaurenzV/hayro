use crate::decode::CombinationOperator;
use crate::error::{ParseError, Result};
use crate::reader::Reader;

#[derive(Debug, Clone)]
pub(crate) struct PageInformation {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) _x_resolution: Option<u32>,
    pub(crate) _y_resolution: Option<u32>,
    pub(crate) flags: PageFlags,
    pub(crate) _striping: PageStriping,
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub(crate) struct PageFlags {
    pub(crate) is_lossless: bool,
    pub(crate) might_contain_refinements: bool,
    pub(crate) default_pixel: u8,
    pub(crate) default_combination_operator: CombinationOperator,
    pub(crate) requires_auxiliary_buffers: bool,
    pub(crate) combination_operator_overridden: bool,
    pub(crate) might_contain_coloured: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PageStriping {
    pub(crate) _is_striped: bool,
    pub(crate) _max_stripe_size: u16,
}

pub(crate) fn parse_page_information(reader: &mut Reader<'_>) -> Result<PageInformation> {
    let width = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    let height = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    let x_resolution_raw = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let x_resolution = if x_resolution_raw == 0 {
        None
    } else {
        Some(x_resolution_raw)
    };

    let y_resolution_raw = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let y_resolution = if y_resolution_raw == 0 {
        None
    } else {
        Some(y_resolution_raw)
    };

    let flags_byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let flags = parse_page_flags(flags_byte);

    let striping_raw = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let striping = PageStriping {
        _is_striped: striping_raw & 0x8000 != 0,
        _max_stripe_size: striping_raw & 0x7FFF,
    };

    Ok(PageInformation {
        width,
        height,
        _x_resolution: x_resolution,
        _y_resolution: y_resolution,
        flags,
        _striping: striping,
    })
}

fn parse_page_flags(flags: u8) -> PageFlags {
    let combo_bits = (flags >> 3) & 0x03;
    let default_combination_operator = match combo_bits {
        0 => CombinationOperator::Or,
        1 => CombinationOperator::And,
        2 => CombinationOperator::Xor,
        3 => CombinationOperator::Xnor,
        _ => unreachable!(),
    };

    PageFlags {
        is_lossless: flags & 0x01 != 0,
        might_contain_refinements: flags & 0x02 != 0,
        default_pixel: (flags >> 2) & 0x01,
        default_combination_operator,
        requires_auxiliary_buffers: flags & 0x20 != 0,
        combination_operator_overridden: flags & 0x40 != 0,
        might_contain_coloured: flags & 0x80 != 0,
    }
}
