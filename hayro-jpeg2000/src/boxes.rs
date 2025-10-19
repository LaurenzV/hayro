use hayro_common::byte::Reader;

/// JP2 signature box - 'jP\040\040'.
pub const JP2_SIGNATURE: u32 = 0x6A502020;
/// File Type box - 'ftyp'.
pub const FILE_TYPE: u32 = 0x66747970;
/// JP2 Header box - 'jp2h'.
pub const JP2_HEADER: u32 = 0x6A703268;
/// Image Header box - 'ihdr'.
pub const IMAGE_HEADER: u32 = 0x69686472;
/// Bits Per Component box - 'bpcc'.
pub const BITS_PER_COMPONENT: u32 = 0x62706363;
/// Colour Specification box - 'colr'.
pub const COLOUR_SPECIFICATION: u32 = 0x636F6C72;
/// Palette box - 'pclr'.
pub const PALETTE: u32 = 0x70636C72;
/// Component Mapping box - 'cmap'.
pub const COMPONENT_MAPPING: u32 = 0x636D6170;
/// Channel Definition box - 'cdef'.
pub const CHANNEL_DEFINITION: u32 = 0x63646566;
/// Resolution box - 'res\x20'.
pub const RESOLUTION: u32 = 0x72657320;
/// Capture Resolution box - 'resc'.
pub const CAPTURE_RESOLUTION: u32 = 0x72657363;
/// Default Display Resolution box - 'resd'.
pub const DISPLAY_RESOLUTION: u32 = 0x72657364;
/// Contiguous Codestream box - 'jp2c'.
pub const CONTIGUOUS_CODESTREAM: u32 = 0x6A703263;
/// Intellectual Property box - 'jp2i'.
pub const INTELLECTUAL_PROPERTY: u32 = 0x6A703269;
/// XML box - 'xml\x20'.
pub const XML: u32 = 0x786D6C20;
/// UUID box - 'uuid'.
pub const UUID: u32 = 0x75756964;
/// UUID Info box - 'uinf'.
pub const UUID_INFO: u32 = 0x75696E66;
/// UUID List box - 'ulst'.
pub const UUID_LIST: u32 = 0x756C7374;
/// URL box - 'url\x20'.
pub const URL: u32 = 0x75726C20;

pub struct Jp2Box<'a> {
    pub data: &'a [u8],
    pub box_type: u32,
}

/// Converts a box tag to its string representation.
///
/// Box tags are stored as 4-byte ASCII codes in big-endian format.
/// For example, 0x66747970 represents "ftyp".
pub fn tag_to_string(tag: u32) -> String {
    let bytes = [
        ((tag >> 24) & 0xFF) as u8,
        ((tag >> 16) & 0xFF) as u8,
        ((tag >> 8) & 0xFF) as u8,
        (tag & 0xFF) as u8,
    ];
    String::from_utf8_lossy(&bytes).to_string()
}

pub fn read_box<'a>(reader: &mut Reader<'a>) -> Option<Jp2Box<'a>> {
    let l_box = reader.read_u32()?;
    let t_box = reader.read_u32()?;

    let data = match l_box {
        // If the value of this field is 0, then the length of the box
        // was not known when the LBox field was written. In this case, this box contains
        // all bytes up to the end of the file.
        0 => {
            let data = reader.tail()?;
            reader.jump_to_end();
            data
        }
        // If the value of this field is 1, then the XLBox field shall exist and the value of
        // that field shall be the actual length of the box.
        // The value includes all of the fields of the box, including the LBox, TBox and XLBox
        // fields.
        1 => {
            let xl_box = reader.read_u64()?.checked_sub(16)?;
            reader.read_bytes(xl_box as usize)?
        }
        // This field specifies the length of the box, stored as a 4-byte big-endian unsigned integer.
        // This value includes all of the fields of the box, including the length and type.
        _ => {
            let length = l_box.checked_sub(8)?;
            reader.read_bytes(length as usize)?
        }
    };

    Some(Jp2Box {
        data,
        box_type: t_box,
    })
}
