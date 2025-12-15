//! File format parsing for JBIG2 bitstreams (Annex D of ITU-T T.88).
//!
//! This module handles parsing of standalone JBIG2 files in both sequential
//! and random-access organization formats.

use crate::reader::Reader;

/// "There are two standalone file organizations possible for a JBIG2 bitstream."
/// (Annex D)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileOrganization {
    /// "This is a standalone file organization. This organization is intended for
    /// streaming applications, where the decoder is guaranteed to begin at the start
    /// of the bitstream and decode everything up to the end of the bitstream."
    /// (D.1)
    Sequential,
    /// "This is a standalone file organization. This organization is intended for
    /// random-access applications, where the decoder might want to process parts of
    /// the file in an arbitrary order." (D.2)
    RandomAccess,
}

/// Parsed file header.
///
/// "A file header contains the following fields, in order:
/// ID string – see D.4.1.
/// File header flags – see D.4.2.
/// Number of pages – see D.4.3." (D.4)
#[derive(Debug, Clone)]
pub(crate) struct FileHeader {
    /// The file organization type.
    pub organization: FileOrganization,
    /// The number of pages in the file, if known.
    pub number_of_pages: Option<u32>,
    /// "If this bit is 0, no generic region segments uses the templates with
    /// 12 AT pixels. If the file contains one or more generic region segments
    /// using such templates, this bit must be 1." (D.4.2, Bit 2)
    pub uses_extended_templates: bool,
    /// "If this bit is 0, no region segment is extended to be coloured. If the
    /// file contains one or more coloured region segments, this bit must be 1."
    /// (D.4.2, Bit 3)
    pub contains_coloured_regions: bool,
}

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

/// A parsed JBIG2 file.
#[derive(Debug)]
pub(crate) struct File<'a> {
    /// The file header.
    pub header: FileHeader,
    /// The segments in the file.
    pub segments: Vec<Segment<'a>>,
}

/// "This is an 8-byte sequence containing 0x97 0x4A 0x42 0x32 0x0D 0x0A 0x1A 0x0A."
/// (D.4.1)
const FILE_HEADER_ID: [u8; 8] = [0x97, 0x4A, 0x42, 0x32, 0x0D, 0x0A, 0x1A, 0x0A];

/// Parse a standalone JBIG2 file.
pub(crate) fn parse_file(data: &[u8]) -> Result<File<'_>, &'static str> {
    let mut reader = Reader::new(data);

    let header = parse_file_header(&mut reader)?;
    let segments = parse_segments(&mut reader, header.organization)?;

    Ok(File { header, segments })
}

fn parse_file_header(reader: &mut Reader<'_>) -> Result<FileHeader, &'static str> {
    // D.4.1: ID string
    let id = reader.read_bytes(8).ok_or("unexpected end of data")?;
    if id != FILE_HEADER_ID {
        return Err("invalid JBIG2 file header ID string");
    }

    // D.4.2: File header flags
    let flags = reader.read_byte().ok_or("unexpected end of data")?;

    // "Bit 0: File organization type. If this bit is 0, the file uses the
    // random-access organization. If this bit is 1, the file uses the
    // sequential organization." (D.4.2)
    let organization = if flags & 0x01 != 0 {
        FileOrganization::Sequential
    } else {
        FileOrganization::RandomAccess
    };

    // "Bit 1: Unknown number of pages. If this bit is 0, then the number of
    // pages contained in the file is known. If this bit is 1, then the number
    // of pages contained in the file was not known at the time that the file
    // header was coded." (D.4.2)
    let unknown_page_count = flags & 0x02 != 0;

    // "Bit 2: If this bit is 0, no generic region segments uses the templates
    // with 12 AT pixels." (D.4.2)
    let uses_extended_templates = flags & 0x04 != 0;

    // "Bit 3: If this bit is 0, no region segment is extended to be coloured."
    // (D.4.2)
    let contains_coloured_regions = flags & 0x08 != 0;

    // "Bits 4-7: Reserved; must be 0." (D.4.2)
    if flags & 0xF0 != 0 {
        return Err("reserved bits in file header flags must be 0");
    }

    // D.4.3: Number of pages
    // "This is a 4-byte field, and is not present if the 'unknown number of
    // pages' bit was 1." (D.4.3)
    let number_of_pages = if unknown_page_count {
        None
    } else {
        Some(reader.read_u32().ok_or("unexpected end of data")?)
    };

    Ok(FileHeader {
        organization,
        number_of_pages,
        uses_extended_templates,
        contains_coloured_regions,
    })
}

fn parse_segments<'a>(
    reader: &mut Reader<'a>,
    organization: FileOrganization,
) -> Result<Vec<Segment<'a>>, &'static str> {
    match organization {
        FileOrganization::Sequential => parse_segments_sequential(reader),
        FileOrganization::RandomAccess => parse_segments_random_access(reader),
    }
}

/// Parse segments in sequential organization.
///
/// "In this organization, the file structure looks like Figure D.1. A file header
/// is followed by a sequence of segments. The two parts of each segment are stored
/// together: first the segment header then the segment data." (D.1)
fn parse_segments_sequential<'a>(reader: &mut Reader<'a>) -> Result<Vec<Segment<'a>>, &'static str> {
    let mut segments = Vec::new();

    loop {
        if reader.remaining() == 0 {
            break;
        }

        let header = parse_segment_header(reader)?;

        // Check for end of file segment.
        let is_eof = matches!(header.segment_type, SegmentType::EndOfFile);

        let data = if let Some(len) = header.data_length {
            reader.read_bytes(len as usize).ok_or("unexpected end of data")?
        } else {
            // Unknown length - need to scan for end marker.
            // This is only valid for immediate lossless generic region.
            return Err("unknown segment data length not yet supported");
        };

        segments.push(Segment { header, data });

        if is_eof {
            break;
        }
    }

    Ok(segments)
}

/// Parse segments in random-access organization.
///
/// "In this organization, the file structure looks like Figure D.2. A file header
/// is followed by a sequence of segments headers; the last segment header is
/// followed by the data for the first segment, then the data for the second
/// segment, and so on." (D.2)
fn parse_segments_random_access<'a>(
    reader: &mut Reader<'a>,
) -> Result<Vec<Segment<'a>>, &'static str> {
    // First, read all segment headers.
    let mut headers = Vec::new();

    loop {
        let header = parse_segment_header(reader)?;
        let is_eof = matches!(header.segment_type, SegmentType::EndOfFile);
        headers.push(header);

        if is_eof {
            break;
        }
    }

    // Then, read all segment data.
    let mut segments = Vec::with_capacity(headers.len());

    for header in headers {
        let data = if let Some(len) = header.data_length {
            reader.read_bytes(len as usize).ok_or("unexpected end of data")?
        } else {
            return Err("unknown segment data length not supported in random-access mode");
        };

        segments.push(Segment { header, data });
    }

    Ok(segments)
}

/// Parse a segment header (Section 7.2).
fn parse_segment_header(reader: &mut Reader<'_>) -> Result<SegmentHeader, &'static str> {
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
