use crate::filter::jbig2::{read_uint16, read_uint32, Jbig2Error, SegmentHeader};
use crate::filter::jbig2::tables::SEGMENT_TYPES;

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
    // When length is unknown, we need to read until end of data or next segment
    if length == 0xffffffff {
        // Implement end-of-segment detection for ImmediateGenericRegion (type 38)
        if segment_type == 38 {
            // For ImmediateGenericRegion with unknown length, we need to find the end pattern
            // by reading ahead to find the region information and create a search pattern
            if position + 17 > data.len() {
                return Err(Jbig2Error::new(
                    "insufficient data for region info in unknown length segment",
                ));
            }

            let region_height = read_uint32(data, position + 4);
            let region_flags = if position + 17 < data.len() {
                data[position + 17]
            } else {
                0
            };
            let mmr = (region_flags & 1) != 0;

            // Create search pattern based on MMR flag and height
            let search_pattern = if mmr {
                // For MMR: just height bytes
                vec![
                    (region_height >> 24) as u8,
                    (region_height >> 16) as u8,
                    (region_height >> 8) as u8,
                    region_height as u8,
                ]
            } else {
                // For non-MMR: 0xff, 0xac followed by height bytes
                vec![
                    0xff,
                    0xac,
                    (region_height >> 24) as u8,
                    (region_height >> 16) as u8,
                    (region_height >> 8) as u8,
                    region_height as u8,
                ]
            };

            // Search for the pattern starting from after the segment header
            let search_start = position + 18; // After region info and flags
            let search_end = data.len().saturating_sub(search_pattern.len());
            let mut found_end = None;

            for i in search_start..=search_end {
                if data[i..i + search_pattern.len()] == search_pattern {
                    found_end = Some(i + search_pattern.len());
                    break;
                }
            }

            let actual_length = if let Some(end_pos) = found_end {
                end_pos - position
            } else {
                // If pattern not found, use remaining data
                data.len() - position
            };

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
        }

        // For other segment types with unknown length, use remaining data
        let remaining_length = data.len() - position;
        return Ok(SegmentHeader {
            number,
            segment_type,
            type_name,
            _deferred_non_retain: deferred_non_retain,
            _retain_bits: retain_bits,
            referred_to,
            _page_association: page_association,
            length: remaining_length as u32,
            header_end: position,
        });
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