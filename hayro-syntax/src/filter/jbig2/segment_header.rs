use crate::filter::jbig2::tables::SEGMENT_TYPES;
use crate::filter::jbig2::{Jbig2Error, SegmentHeader, read_uint16, read_uint32};

// Segment header reading - ported from readSegmentHeader function
pub(crate) fn read_segment_header(data: &[u8], start: usize) -> Result<SegmentHeader, Jbig2Error> {
    let number = read_uint32(data, start);
    let flags = data[start + 4];
    let segment_type = flags & 0x3f;

    if segment_type as usize >= SEGMENT_TYPES.len()
        || SEGMENT_TYPES[segment_type as usize].is_none()
    {
        return Err(Jbig2Error::new(&format!(
            "invalid segment type: {}",
            segment_type
        )));
    }

    let type_name = SEGMENT_TYPES[segment_type as usize].unwrap().to_string();
    let deferred_non_retain = (flags & 0x80) != 0;
    let page_association_field_size = (flags & 0x40) != 0;

    let referred_flags = data[start + 5];
    let mut referred_to_count = ((referred_flags >> 5) & 7) as usize;
    let mut retain_bits = vec![referred_flags & 31];
    let mut position = start + 6;

    if referred_flags == 7 {
        referred_to_count = (read_uint32(data, position - 1) & 0x1fffffff) as usize;
        position += 3;
        let mut bytes = (referred_to_count + 7) >> 3;
        retain_bits[0] = data[position];
        position += 1;
        bytes -= 1;
        while bytes > 0 && position < data.len() {
            retain_bits.push(data[position]);
            position += 1;
            bytes -= 1;
        }
    } else if referred_flags == 5 || referred_flags == 6 {
        return Err(Jbig2Error::new("invalid referred-to flags"));
    }

    let referred_to_segment_number_size = if number <= 256 {
        1
    } else if number <= 65536 {
        2
    } else {
        4
    };

    let mut referred_to = Vec::new();
    for _ in 0..referred_to_count {
        if position + referred_to_segment_number_size > data.len() {
            return Err(Jbig2Error::new(
                "insufficient data for referred-to segments",
            ));
        }

        let number = match referred_to_segment_number_size {
            1 => data[position] as u32,
            2 => read_uint16(data, position) as u32,
            4 => read_uint32(data, position),
            _ => return Err(Jbig2Error::new("invalid segment number size")),
        };
        referred_to.push(number);
        position += referred_to_segment_number_size;
    }

    let page_association = if !page_association_field_size {
        if position >= data.len() {
            return Err(Jbig2Error::new("insufficient data for page association"));
        }
        data[position] as u32
    } else {
        if position + 4 > data.len() {
            return Err(Jbig2Error::new("insufficient data for page association"));
        }
        read_uint32(data, position)
    };
    position += if page_association_field_size { 4 } else { 1 };

    if position + 4 > data.len() {
        return Err(Jbig2Error::new("insufficient data for segment length"));
    }
    let length = read_uint32(data, position);
    position += 4;

    // Handle unknown segment length (0xffffffff) cases
    if length == 0xffffffff {
        // 7.2.7 Segment data length, unknown segment length
        if segment_type == 38 {
            // ImmediateGenericRegion
            let generic_region_info = super::read_region_segment_information(data, position)?;
            let region_segment_information_field_length = 17;
            let generic_region_segment_flags =
                data[position + region_segment_information_field_length];
            let generic_region_mmr = (generic_region_segment_flags & 1) != 0;

            // Searching for the segment end
            let search_pattern_length = 6;
            let mut search_pattern = vec![0u8; search_pattern_length];
            if !generic_region_mmr {
                search_pattern[0] = 0xff;
                search_pattern[1] = 0xac;
            }
            search_pattern[2] = (generic_region_info.height >> 24) as u8;
            search_pattern[3] = (generic_region_info.height >> 16) as u8;
            search_pattern[4] = (generic_region_info.height >> 8) as u8;
            search_pattern[5] = generic_region_info.height as u8;

            let mut found_length = None;
            for i in position..data.len() {
                let mut j = 0;
                while j < search_pattern_length && search_pattern[j] == data[i + j] {
                    j += 1;
                }
                if j == search_pattern_length {
                    found_length = Some(i + search_pattern_length);
                    break;
                }
            }

            let actual_length =
                found_length.ok_or_else(|| Jbig2Error::new("segment end was not found"))?;

            return Ok(SegmentHeader {
                number,
                segment_type,
                type_name,
                _deferred_non_retain: deferred_non_retain,
                _retain_bits: retain_bits,
                referred_to,
                _page_association: page_association,
                length: actual_length as u32,
                header_end: position,
            });
        } else {
            return Err(Jbig2Error::new("invalid unknown segment length"));
        }
    }

    Ok(SegmentHeader {
        number,
        segment_type,
        type_name,
        _deferred_non_retain: deferred_non_retain,
        _retain_bits: retain_bits,
        referred_to,
        _page_association: page_association,
        length,
        header_end: position,
    })
}
