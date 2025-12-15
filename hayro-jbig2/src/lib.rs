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

/// Temporary debug function to test file parsing.
pub fn debug_parse_file(data: &[u8]) {
    use file::{FileOrganization, parse_file};

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
            for (i, segment) in f.segments.iter().enumerate() {
                println!(
                    "[{i}] Segment #{}: type={:?}, page={}, data_len={}, referred_to={:?}",
                    segment.header.segment_number,
                    segment.header.segment_type,
                    segment.header.page_association,
                    segment.data.len(),
                    segment.header.referred_to_segments,
                );
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
