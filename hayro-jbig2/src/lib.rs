/*!
A memory-safe, pure-Rust JBIG2 decoder.

`hayro-jbig2` decodes JBIG2 images as specified in ITU-T T.88 (also known as
ISO/IEC 14492). JBIG2 is a bi-level image compression standard commonly used
in PDF documents for compressing scanned text documents.

# Example
```rust,no_run
use hayro_jbig2::decode;

let data = std::fs::read("image.jb2").unwrap();
let image = decode(&data, None).unwrap();

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
    use file::{FileOrganization, parse_file};
    use reader::Reader;
    use segment::generic_region::parse_generic_region_header;
    use segment::page_info::parse_page_information;
    use segment::SegmentType;

    match parse_file(data) {
        Ok(f) => {
            println!("=== File Header ===");
            println!(
                "Organization: {:?}",
                match f.header.organization {
                    FileOrganization::Sequential => "Sequential",
                    FileOrganization::RandomAccess => "Random-access",
                }
            );
            println!("Number of pages: {:?}", f.header.number_of_pages);
            println!(
                "Uses extended templates: {}",
                f.header.uses_extended_templates
            );
            println!("Contains coloured regions: {}", f.header.contains_coloured_regions);
            println!();

            println!("=== Segments ({} total) ===", f.segments.len());
            for (i, seg) in f.segments.iter().enumerate() {
                println!(
                    "[{i}] Segment #{}: type={:?}, page={}, data_len={}, referred_to={:?}",
                    seg.header.segment_number,
                    seg.header.segment_type,
                    seg.header.page_association,
                    seg.data.len(),
                    seg.header.referred_to_segments,
                );

                // Parse and print segment-specific data
                let mut reader = Reader::new(seg.data);
                match seg.header.segment_type {
                    SegmentType::PageInformation => {
                        match parse_page_information(&mut reader) {
                            Ok(info) => {
                                println!(
                                    "    Page: {}x{}, default_pixel={}, combo={:?}",
                                    info.width,
                                    info.height,
                                    info.flags.default_pixel,
                                    info.flags.default_combination_operator,
                                );
                                if info.striping.is_striped {
                                    println!(
                                        "    Striped: max_stripe_size={}",
                                        info.striping.max_stripe_size
                                    );
                                }
                            }
                            Err(e) => {
                                println!("    Error parsing page information: {e}");
                            }
                        }
                    }
                    SegmentType::IntermediateGenericRegion
                    | SegmentType::ImmediateGenericRegion
                    | SegmentType::ImmediateLosslessGenericRegion => {
                        match parse_generic_region_header(&mut reader) {
                            Ok(header) => {
                                println!(
                                    "    Region: {}x{} at ({}, {}), combo={:?}",
                                    header.region_info.width,
                                    header.region_info.height,
                                    header.region_info.x_location,
                                    header.region_info.y_location,
                                    header.region_info.combination_operator,
                                );
                                println!(
                                    "    Flags: MMR={}, GBTEMPLATE={}, TPGDON={}, EXTTEMPLATE={}",
                                    header.mmr, header.gb_template, header.tpgdon, header.ext_template,
                                );
                                if !header.adaptive_template_pixels.is_empty() {
                                    let at_str: Vec<String> = header
                                        .adaptive_template_pixels
                                        .iter()
                                        .map(|p| format!("({}, {})", p.x, p.y))
                                        .collect();
                                    println!("    AT pixels: {}", at_str.join(", "));
                                }
                            }
                            Err(e) => {
                                println!("    Error parsing generic region header: {e}");
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Err(e) => {
            eprintln!("Error parsing JBIG2 file: {e}");
        }
    }
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

/// Global segment data that can be shared across multiple JBIG2 streams.
///
/// In PDF, JBIG2 images can reference global segments stored separately.
/// This struct holds the decoded global segment data.
#[derive(Debug, Clone, Default)]
pub struct Globals {
    /// The raw global segment data.
    pub data: Vec<u8>,
}

impl Globals {
    /// Create new globals from raw data.
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

/// Decode a JBIG2 image.
///
/// # Arguments
/// * `data` - The JBIG2 data to decode.
/// * `globals` - Optional global segments (used in PDF context).
///
/// # Returns
/// The decoded image, or an error message if decoding failed.
pub fn decode(data: &[u8], globals: Option<&Globals>) -> Result<Image, &'static str> {
    let _ = (data, globals);
    Err("JBIG2 decoding not yet implemented")
}
