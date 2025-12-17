/*!
A memory-safe, pure-Rust JBIG2 decoder.

`hayro-jbig2` decodes JBIG2 images as specified in ITU-T T.88 (also known as
ISO/IEC 14492). JBIG2 is a bi-level image compression standard commonly used
in PDF documents for compressing scanned text documents.

# Example
```rust,no_run
use hayro_jbig2::decode;

let data = std::fs::read("image.jb2").unwrap();
let image = decode(&data).unwrap();

println!("{}x{} image", image.width, image.height);
```

# Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![forbid(unsafe_code)]
#![allow(missing_docs)]

pub(crate) mod file;
pub(crate) mod reader;
pub(crate) mod segment;

/// Temporary debug function to test file parsing.
pub fn debug_parse_file(data: &[u8]) {
    use file::parse_file;

    match parse_file(data) {
        Ok(f) => {
            print_header_debug(&f.header);
            println!();

            println!("=== Segments ({} total) ===", f.segments.len());
            for (i, seg) in f.segments.iter().enumerate() {
                print_segment_debug(i, seg);
            }
        }
        Err(e) => {
            eprintln!("Error parsing JBIG2 file: {e}");
        }
    }
}

/// Print debug information for the file header.
fn print_header_debug(header: &file::FileHeader) {
    use file::FileOrganization;

    println!("=== File Header ===");
    println!(
        "Organization: {:?}",
        match header.organization {
            FileOrganization::Sequential => "Sequential",
            FileOrganization::RandomAccess => "Random-access",
        }
    );
    println!("Number of pages: {:?}", header.number_of_pages);
    println!(
        "Uses extended templates: {}",
        header.uses_extended_templates
    );
    println!(
        "Contains coloured regions: {}",
        header.contains_coloured_regions
    );
}

/// Print debug information for a single segment.
fn print_segment_debug(index: usize, seg: &segment::Segment<'_>) -> Result<(), &'static str> {
    use reader::Reader;
    use segment::SegmentType;
    use segment::generic_region::parse_generic_region_header;
    use segment::page_info::parse_page_information;

    println!(
        "[{index}] Segment #{}: type={:?}, page={}, data_len={}, referred_to={:?}",
        seg.header.segment_number,
        seg.header.segment_type,
        seg.header.page_association,
        seg.data.len(),
        seg.header.referred_to_segments,
    );

    let mut ctx = Err("region segment appeared before page information.");

    // Parse and print segment-specific data
    let mut reader = Reader::new(seg.data);
    match seg.header.segment_type {
        SegmentType::PageInformation => {
            let page_info = parse_page_information(&mut reader)?;
            ctx = Ok(DecoderContext {
                width: page_info.width,
                height: page_info.height,
                data: vec![0; page_info.width as usize * page_info.height as usize],
            });
        }
        SegmentType::IntermediateGenericRegion
        | SegmentType::ImmediateGenericRegion
        | SegmentType::ImmediateLosslessGenericRegion => {
            let header = parse_generic_region_header(&mut reader)?;
            eprintln!("{:?}", header);
        }
        _ => {}
    }

    Ok(())
}

pub(crate) struct DecoderContext {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) data: Vec<u8>,
}

/// A decoded JBIG2 image.
#[derive(Debug, Clone)]
pub struct Image {
    /// The width of the image in pixels.
    pub width: u32,
    /// The height of the image in pixels.
    pub height: u32,
    /// The raw pixel data (1 bit per pixel, packed into bytes).
    /// Each row is byte-aligned.
    pub data: Vec<u8>,
}

impl Image {
    /// Returns the stride (number of bytes per row) of the image.
    pub fn stride(&self) -> usize {
        (self.width as usize + 7) / 8
    }
}
