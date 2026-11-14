use alloc::vec::Vec;

use crate::error::{ParseError, Result, SegmentError, bail, err};
use crate::reader::Reader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentType {
    SymbolDictionary,
    IntermediateTextRegion,
    ImmediateTextRegion,
    ImmediateLosslessTextRegion,
    PatternDictionary,
    IntermediateHalftoneRegion,
    ImmediateHalftoneRegion,
    ImmediateLosslessHalftoneRegion,
    IntermediateGenericRegion,
    ImmediateGenericRegion,
    ImmediateLosslessGenericRegion,
    IntermediateGenericRefinementRegion,
    ImmediateGenericRefinementRegion,
    ImmediateLosslessGenericRefinementRegion,
    PageInformation,
    EndOfPage,
    EndOfStripe,
    EndOfFile,
    Profiles,
    Tables,
    ColourPalette,
    Extension,
    Reserved,
}

impl SegmentType {
    pub(crate) fn requires_page(self) -> bool {
        self.is_immediate_region()
            || matches!(
                self,
                Self::IntermediateTextRegion
                    | Self::IntermediateHalftoneRegion
                    | Self::IntermediateGenericRegion
                    | Self::IntermediateGenericRefinementRegion
                    | Self::PageInformation
                    | Self::EndOfPage
                    | Self::EndOfStripe
            )
    }

    pub(crate) fn is_immediate_region(self) -> bool {
        matches!(
            self,
            Self::ImmediateTextRegion
                | Self::ImmediateLosslessTextRegion
                | Self::ImmediateHalftoneRegion
                | Self::ImmediateLosslessHalftoneRegion
                | Self::ImmediateGenericRegion
                | Self::ImmediateLosslessGenericRegion
                | Self::ImmediateGenericRefinementRegion
                | Self::ImmediateLosslessGenericRefinementRegion
        )
    }

    fn from_type_value(value: u8) -> Self {
        match value {
            0 => Self::SymbolDictionary,
            4 => Self::IntermediateTextRegion,
            6 => Self::ImmediateTextRegion,
            7 => Self::ImmediateLosslessTextRegion,
            16 => Self::PatternDictionary,
            20 => Self::IntermediateHalftoneRegion,
            22 => Self::ImmediateHalftoneRegion,
            23 => Self::ImmediateLosslessHalftoneRegion,
            36 => Self::IntermediateGenericRegion,
            38 => Self::ImmediateGenericRegion,
            39 => Self::ImmediateLosslessGenericRegion,
            40 => Self::IntermediateGenericRefinementRegion,
            42 => Self::ImmediateGenericRefinementRegion,
            43 => Self::ImmediateLosslessGenericRefinementRegion,
            48 => Self::PageInformation,
            49 => Self::EndOfPage,
            50 => Self::EndOfStripe,
            51 => Self::EndOfFile,
            52 => Self::Profiles,
            53 => Self::Tables,
            54 => Self::ColourPalette,
            62 => Self::Extension,
            _ => Self::Reserved,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SegmentHeader {
    pub(crate) segment_number: u32,
    pub(crate) segment_type: SegmentType,
    pub(crate) _retain_flag: bool,
    pub(crate) page_association: u32,
    pub(crate) referred_to_segments: Vec<u32>,
    pub(crate) data_length: Option<u32>,
}

#[derive(Debug)]
pub(crate) struct Segment<'a> {
    pub(crate) header: SegmentHeader,
    pub(crate) data: &'a [u8],
}

pub(crate) fn parse_segment_header(reader: &mut Reader<'_>, strict: bool) -> Result<SegmentHeader> {
    let segment_number = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    let segment_type = SegmentType::from_type_value(flags & 0x3F);

    if strict && segment_type == SegmentType::Reserved {
        bail!(SegmentError::UnknownType);
    }

    let page_association_long = flags & 0x40 != 0;

    let retain_flag = flags & 0x80 == 0;

    let count_and_retention = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let short_count = (count_and_retention >> 5) & 0x07;

    if short_count == 5 || short_count == 6 {
        bail!(SegmentError::InvalidReferredCount);
    }

    let referred_to_count = if short_count < 7 {
        short_count as u32
    } else {
        let rest = reader.read_bytes(3).ok_or(ParseError::UnexpectedEof)?;
        u32::from_be_bytes([count_and_retention & 0x1F, rest[0], rest[1], rest[2]])
    };

    if short_count == 7 {
        let retention_bytes = (referred_to_count as usize + 1).div_ceil(8);
        reader
            .skip_bytes(retention_bytes)
            .ok_or(ParseError::UnexpectedEof)?;
    }

    let number_size = if segment_number <= 256 {
        1
    } else if segment_number <= 65536 {
        2
    } else {
        4
    };
    let numbers = (referred_to_count as usize)
        .checked_mul(number_size)
        .and_then(|len| reader.read_bytes(len))
        .ok_or(ParseError::UnexpectedEof)?;

    let mut referred_to_segments = Vec::with_capacity(referred_to_count as usize);
    for bytes in numbers.chunks_exact(number_size) {
        let referred = bytes
            .iter()
            .fold(0, |acc, &byte| (acc << 8) | u32::from(byte));

        if referred >= segment_number {
            bail!(SegmentError::InvalidReference);
        }

        referred_to_segments.push(referred);
    }

    let page_association = if page_association_long {
        reader.read_u32().ok_or(ParseError::UnexpectedEof)?
    } else {
        reader.read_byte().ok_or(ParseError::UnexpectedEof)? as u32
    };

    let data_length_raw = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let data_length = if data_length_raw == 0xFFFFFFFF {
        if strict && segment_type != SegmentType::ImmediateGenericRegion {
            bail!(SegmentError::InvalidUnknownLength);
        }

        None
    } else {
        Some(data_length_raw)
    };

    Ok(SegmentHeader {
        segment_number,
        segment_type,
        _retain_flag: retain_flag,
        page_association,
        referred_to_segments,
        data_length,
    })
}

pub(crate) fn parse_segment<'a>(reader: &mut Reader<'a>, strict: bool) -> Result<Segment<'a>> {
    let header = parse_segment_header(reader, strict)?;
    parse_segment_data(reader, header)
}

pub(crate) fn parse_segment_data<'a>(
    reader: &mut Reader<'a>,
    header: SegmentHeader,
) -> Result<Segment<'a>> {
    let data = if let Some(len) = header.data_length {
        reader
            .read_bytes(len as usize)
            .ok_or(ParseError::UnexpectedEof)?
    } else {
        let len = scan_for_immediate_generic_region_size(reader)?;
        reader.read_bytes(len).ok_or(ParseError::UnexpectedEof)?
    };

    Ok(Segment { header, data })
}

fn scan_for_immediate_generic_region_size(reader: &Reader<'_>) -> Result<usize> {
    let mut scan = reader.clone();
    let start_offset = scan.byte_pos();

    scan.skip_bytes(17).ok_or(ParseError::UnexpectedEof)?;
    let flags = scan.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let uses_mmr = (flags & 1) != 0;

    // The AT flags can contain the end marker, so the search starts after them.
    let at_bytes = match (uses_mmr, (flags >> 1) & 0x03, flags & 0x10 != 0) {
        (true, _, _) => 0,
        (false, 0, false) => 8,
        (false, 0, true) => 24,
        (false, _, _) => 2,
    };
    scan.skip_bytes(at_bytes).ok_or(ParseError::UnexpectedEof)?;

    let end_marker: [u8; 2] = if uses_mmr { [0x00, 0x00] } else { [0xFF, 0xAC] };

    while let Some(bytes) = scan.peek_bytes(6) {
        if bytes[..2] == end_marker {
            return Ok(scan.byte_pos() - start_offset + 2 + 4);
        }
        scan.skip_bytes(1).ok_or(ParseError::UnexpectedEof)?;
    }

    err!(SegmentError::MissingEndMarker)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn test_segment_header_example_1() {
        let data = [
            0x00, 0x00, 0x00, 0x20, 0x86, 0x6B, 0x02, 0x1E, 0x05, 0x04, 0x00, 0x00, 0x00, 0x10,
        ];

        let mut reader = Reader::new(&data);
        let header = parse_segment_header(&mut reader, true).unwrap();

        assert_eq!(header.segment_number, 32);

        assert_eq!(header.segment_type, SegmentType::ImmediateTextRegion);
        assert!(!header._retain_flag);

        assert_eq!(header.referred_to_segments, vec![2, 30, 5]);

        assert_eq!(header.page_association, 4);

        assert_eq!(header.data_length, Some(16));
    }

    #[test]
    fn test_segment_header_example_2() {
        #[rustfmt::skip]
        let data = [
            0x00, 0x00, 0x02, 0x34,
            0x40,
            0xE0, 0x00, 0x00, 0x09,
            0x02, 0xFD,
            0x01, 0x00,
            0x00, 0x02,
            0x00, 0x1E,
            0x00, 0x05,
            0x02, 0x00,
            0x02, 0x01,
            0x02, 0x02,
            0x02, 0x03,
            0x02, 0x04,
            0x00, 0x00, 0x04, 0x01,
            0x00, 0x00, 0x00, 0x20,
        ];

        let mut reader = Reader::new(&data);
        let header = parse_segment_header(&mut reader, true).unwrap();

        assert_eq!(header.segment_number, 564);

        assert_eq!(header.segment_type, SegmentType::SymbolDictionary);
        assert!(header._retain_flag);

        assert_eq!(
            header.referred_to_segments,
            vec![256, 2, 30, 5, 512, 513, 514, 515, 516]
        );

        assert_eq!(header.page_association, 1025);

        assert_eq!(header.data_length, Some(32));
    }
}
