//! Segment parsing for JBIG2 bitstreams (Section 7.2).
//!
//! This module handles parsing of individual segment headers and defines
//! the segment types used in JBIG2.

use crate::reader::Reader;

/// Segment types as defined in Table 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentType {
    /// Symbol dictionary segment (type 0).
    SymbolDictionary,
    /// Intermediate text region segment (type 4).
    IntermediateTextRegion,
    /// Immediate text region segment (type 6).
    ImmediateTextRegion,
    /// Immediate lossless text region segment (type 7).
    ImmediateLosslessTextRegion,
    /// Pattern dictionary segment (type 16).
    PatternDictionary,
    /// Intermediate halftone region segment (type 20).
    IntermediateHalftoneRegion,
    /// Immediate halftone region segment (type 22).
    ImmediateHalftoneRegion,
    /// Immediate lossless halftone region segment (type 23).
    ImmediateLosslessHalftoneRegion,
    /// Intermediate generic region segment (type 36).
    IntermediateGenericRegion,
    /// Immediate generic region segment (type 38).
    ImmediateGenericRegion,
    /// Immediate lossless generic region segment (type 39).
    ImmediateLosslessGenericRegion,
    /// Intermediate generic refinement region segment (type 40).
    IntermediateGenericRefinementRegion,
    /// Immediate generic refinement region segment (type 42).
    ImmediateGenericRefinementRegion,
    /// Immediate lossless generic refinement region segment (type 43).
    ImmediateLosslessGenericRefinementRegion,
    /// Page information segment (type 48).
    PageInformation,
    /// End of page segment (type 49).
    EndOfPage,
    /// End of stripe segment (type 50).
    EndOfStripe,
    /// End of file segment (type 51).
    EndOfFile,
    /// Profiles segment (type 52).
    Profiles,
    /// Tables segment (type 53).
    Tables,
    /// Colour palette segment (type 54).
    ColourPalette,
    /// Extension segment (type 62).
    Extension,
    /// Unknown or reserved segment type.
    Unknown(u8),
}

impl SegmentType {
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
            _ => Self::Unknown(value),
        }
    }
}

/// A parsed segment header.
#[derive(Debug, Clone)]
pub(crate) struct SegmentHeader {
    /// The segment number.
    pub segment_number: u32,
    /// The segment type.
    pub segment_type: SegmentType,
    /// Whether this segment's data should be retained after decoding.
    pub retain_flag: bool,
    /// The page this segment is associated with (0 means not associated with any page).
    pub page_association: u32,
    /// The segment numbers this segment refers to.
    pub referred_to_segments: Vec<u32>,
    /// The length of the segment data. `None` means unknown length (only valid
    /// for immediate lossless generic region in sequential organization).
    pub data_length: Option<u32>,
}

/// A parsed segment with its header and data.
#[derive(Debug)]
pub(crate) struct Segment<'a> {
    /// The segment header.
    pub header: SegmentHeader,
    /// The segment data (borrowed slice).
    pub data: &'a [u8],
}

/// Parse a segment header (Section 7.2).
pub(crate) fn parse_segment_header(reader: &mut Reader<'_>) -> Result<SegmentHeader, &'static str> {
    // 7.2.2: Segment number
    let segment_number = reader.read_u32().ok_or("unexpected end of data")?;

    // 7.2.3: Segment header flags
    let flags = reader.read_byte().ok_or("unexpected end of data")?;

    // Bits 0-5: Segment type
    let segment_type = SegmentType::from_type_value(flags & 0x3F);

    // Bit 6: Page association size flag (0 = 1 byte, 1 = 4 bytes)
    let page_association_long = flags & 0x40 != 0;

    // Bit 7: Deferred non-retain flag
    let retain_flag = flags & 0x80 == 0;

    // 7.2.4: Referred-to segment count and retention flags
    let count_and_retention = reader.read_byte().ok_or("unexpected end of data")?;
    let short_count = (count_and_retention >> 5) & 0x07;

    let referred_to_count = if short_count < 7 {
        short_count as u32
    } else {
        // Long form: next 4 bytes contain the count.
        // First, read 3 more bytes to complete the 4-byte count field.
        let b1 = count_and_retention & 0x1F;
        let b2 = reader.read_byte().ok_or("unexpected end of data")?;
        let b3 = reader.read_byte().ok_or("unexpected end of data")?;
        let b4 = reader.read_byte().ok_or("unexpected end of data")?;
        u32::from_be_bytes([b1, b2, b3, b4])
    };

    // 7.2.5: Referred-to segment numbers
    let segment_number_size = if segment_number <= 255 {
        1
    } else if segment_number <= 65535 {
        2
    } else {
        4
    };

    let mut referred_to_segments = Vec::with_capacity(referred_to_count as usize);
    for _ in 0..referred_to_count {
        let referred = match segment_number_size {
            1 => reader.read_byte().ok_or("unexpected end of data")? as u32,
            2 => reader.read_u16().ok_or("unexpected end of data")? as u32,
            4 => reader.read_u32().ok_or("unexpected end of data")?,
            _ => unreachable!(),
        };
        referred_to_segments.push(referred);
    }

    // 7.2.6: Segment page association
    let page_association = if page_association_long {
        reader.read_u32().ok_or("unexpected end of data")?
    } else {
        reader.read_byte().ok_or("unexpected end of data")? as u32
    };

    // 7.2.7: Segment data length
    let data_length_raw = reader.read_u32().ok_or("unexpected end of data")?;
    let data_length = if data_length_raw == 0xFFFFFFFF {
        None // Unknown length
    } else {
        Some(data_length_raw)
    };

    Ok(SegmentHeader {
        segment_number,
        segment_type,
        retain_flag,
        page_association,
        referred_to_segments,
        data_length,
    })
}
