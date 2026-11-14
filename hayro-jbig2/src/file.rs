use alloc::vec::Vec;

use crate::error::{DecodeError, FormatError, ParseError, Result, bail};
use crate::reader::Reader;
use crate::segment::{
    Segment, SegmentType, parse_segment, parse_segment_data, parse_segment_header,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileOrganization {
    Sequential,
    RandomAccess,
}

#[derive(Debug, Clone)]
pub(crate) struct FileHeader {
    pub(crate) organization: FileOrganization,
    pub(crate) number_of_pages: Option<u32>,
    pub(crate) _uses_extended_templates: bool,
    pub(crate) _contains_coloured_regions: bool,
}

#[derive(Debug)]
pub(crate) struct File<'a> {
    pub(crate) header: FileHeader,
    pub(crate) segments: Vec<Segment<'a>>,
}

const FILE_HEADER_ID: [u8; 8] = [0x97, 0x4A, 0x42, 0x32, 0x0D, 0x0A, 0x1A, 0x0A];

pub(crate) fn parse_file(data: &[u8], strict: bool) -> Result<File<'_>> {
    let mut reader = Reader::new(data);

    let header = parse_file_header(&mut reader)?;
    let mut segments = parse_segments(&mut reader, header.organization, strict)?;

    // The spec requires sorted segments; sort anyway since lookups binary-search them.
    segments.sort_by_key(|seg| seg.header.segment_number);

    Ok(File { header, segments })
}

fn parse_file_header(reader: &mut Reader<'_>) -> Result<FileHeader> {
    let id = reader.read_bytes(8).ok_or(ParseError::UnexpectedEof)?;
    if id != FILE_HEADER_ID {
        bail!(FormatError::InvalidHeader);
    }

    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    let organization = if flags & 0x01 != 0 {
        FileOrganization::Sequential
    } else {
        FileOrganization::RandomAccess
    };

    let unknown_page_count = flags & 0x02 != 0;

    let uses_extended_templates = flags & 0x04 != 0;

    let contains_coloured_regions = flags & 0x08 != 0;

    if flags & 0xF0 != 0 {
        bail!(FormatError::ReservedBits);
    }

    let number_of_pages = if unknown_page_count {
        None
    } else {
        Some(reader.read_u32().ok_or(ParseError::UnexpectedEof)?)
    };

    Ok(FileHeader {
        organization,
        number_of_pages,
        _uses_extended_templates: uses_extended_templates,
        _contains_coloured_regions: contains_coloured_regions,
    })
}

fn parse_segments<'a>(
    reader: &mut Reader<'a>,
    organization: FileOrganization,
    strict: bool,
) -> Result<Vec<Segment<'a>>> {
    let mut segments = Vec::new();

    match organization {
        FileOrganization::Sequential => parse_segments_sequential(reader, &mut segments, strict)?,
        FileOrganization::RandomAccess => {
            parse_segments_random_access(reader, &mut segments, strict)?;
        }
    }

    Ok(segments)
}

pub(crate) fn parse_segments_sequential<'a>(
    reader: &mut Reader<'a>,
    segments: &mut Vec<Segment<'a>>,
    strict: bool,
) -> Result<()> {
    loop {
        if reader.at_end() {
            break;
        }

        let segment = match parse_segment(reader, strict) {
            Ok(segment) => segment,
            Err(DecodeError::Parse(ParseError::UnexpectedEof)) if !strict => break,
            Err(e) => return Err(e),
        };

        let is_eof = matches!(segment.header.segment_type, SegmentType::EndOfFile);
        segments.push(segment);

        if is_eof {
            break;
        }
    }

    Ok(())
}

fn parse_segments_random_access<'a>(
    reader: &mut Reader<'a>,
    segments: &mut Vec<Segment<'a>>,
    strict: bool,
) -> Result<()> {
    let mut headers = Vec::new();

    loop {
        if reader.at_end() {
            break;
        }

        let header = parse_segment_header(reader, strict)?;

        let is_eof = matches!(header.segment_type, SegmentType::EndOfFile);
        headers.push(header);

        if is_eof {
            break;
        }
    }

    for header in headers {
        segments.push(parse_segment_data(reader, header)?);
    }

    Ok(())
}
