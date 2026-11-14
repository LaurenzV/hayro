#![allow(
    dead_code,
    reason = "JP2 box constants exist for completeness but not all are referenced yet"
)]

use alloc::string::{String, ToString};

use crate::reader::BitReader;

pub(crate) const JP2_SIGNATURE: u32 = 0x6A502020;
pub(crate) const FILE_TYPE: u32 = 0x66747970;
pub(crate) const JP2_HEADER: u32 = 0x6A703268;
pub(crate) const IMAGE_HEADER: u32 = 0x69686472;
pub(crate) const BITS_PER_COMPONENT: u32 = 0x62706363;
pub(crate) const COLOUR_SPECIFICATION: u32 = 0x636F6C72;
pub(crate) const PALETTE: u32 = 0x70636C72;
pub(crate) const COMPONENT_MAPPING: u32 = 0x636D6170;
pub(crate) const CHANNEL_DEFINITION: u32 = 0x63646566;
pub(crate) const RESOLUTION: u32 = 0x72657320;
pub(crate) const CAPTURE_RESOLUTION: u32 = 0x72657363;
pub(crate) const DISPLAY_RESOLUTION: u32 = 0x72657364;
pub(crate) const CONTIGUOUS_CODESTREAM: u32 = 0x6A703263;
pub(crate) const INTELLECTUAL_PROPERTY: u32 = 0x6A703269;
pub(crate) const XML: u32 = 0x786D6C20;
pub(crate) const UUID: u32 = 0x75756964;
pub(crate) const UUID_INFO: u32 = 0x75696E66;
pub(crate) const UUID_LIST: u32 = 0x756C7374;
pub(crate) const URL: u32 = 0x75726C20;

pub(crate) struct Jp2Box<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) box_type: u32,
}

pub(crate) fn tag_to_string(tag: u32) -> String {
    let bytes = [
        ((tag >> 24) & 0xFF) as u8,
        ((tag >> 16) & 0xFF) as u8,
        ((tag >> 8) & 0xFF) as u8,
        (tag & 0xFF) as u8,
    ];
    String::from_utf8_lossy(&bytes).to_string()
}

pub(crate) fn read<'a>(reader: &mut BitReader<'a>, strict: bool) -> Option<Jp2Box<'a>> {
    let l_box = reader.read_u32()?;
    let t_box = reader.read_u32()?;

    let data = match l_box {
        0 => {
            let data = reader.tail()?;
            reader.jump_to_end();
            data
        }
        1 => {
            let xl_box = reader.read_u64()?.checked_sub(16)?;
            read_data(reader, xl_box as usize, strict)?
        }
        _ => {
            let length = l_box.checked_sub(8)?;
            read_data(reader, length as usize, strict)?
        }
    };

    Some(Jp2Box {
        data,
        box_type: t_box,
    })
}

fn read_data<'a>(reader: &mut BitReader<'a>, length: usize, strict: bool) -> Option<&'a [u8]> {
    if let Some(data) = reader.read_bytes(length) {
        return Some(data);
    }

    // Some files declare a box length that exceeds the file, in which case
    // we use the remaining data instead.
    if strict {
        return None;
    }

    let data = reader.tail()?;
    reader.jump_to_end();

    Some(data)
}
