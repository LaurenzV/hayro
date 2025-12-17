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

mod arithmetic_decoder;
mod bitmap;
mod file;
mod reader;
mod segment;

use bitmap::Bitmap;
use file::parse_file;
use reader::Reader;
use segment::SegmentType;
use segment::generic_region::decode_generic_region;
use segment::page_info::{PageInformation, parse_page_information};

/// A decoded JBIG2 image.
#[derive(Debug, Clone)]
pub struct Image {
    /// The width of the image in pixels.
    pub width: u32,
    /// The height of the image in pixels.
    pub height: u32,
    /// The raw pixel data, one bool per pixel, row-major order.
    /// `true` means black, `false` means white.
    pub data: Vec<bool>,
}

/// Decode a JBIG2 image from the given data.
///
/// This function parses and decodes a standalone JBIG2 file, returning the
/// decoded bitmap image.
///
/// # Example
/// ```rust,no_run
/// let data = std::fs::read("image.jb2").unwrap();
/// let image = hayro_jbig2::decode(&data).unwrap();
/// println!("{}x{} image", image.width, image.height);
/// ```
pub fn decode(data: &[u8]) -> Result<Image, &'static str> {
    let file = parse_file(data)?;

    let mut ctx: Result<DecodeContext, &'static str> = Err("attempted to decode\
    region before page information appeared");

    for seg in &file.segments {
        let mut reader = Reader::new(seg.data);

        match seg.header.segment_type {
            // "Page information – see 7.4.8." (type 48)
            SegmentType::PageInformation => ctx = Ok(get_ctx(&mut reader)?),
            SegmentType::ImmediateLosslessGenericRegion => {
                decode_generic_region(ctx.as_mut().map_err(|e| *e)?, &mut reader)?;
            }

            // End of page - we're done with this page.
            SegmentType::EndOfPage | SegmentType::EndOfFile => {
                break;
            }

            // Other segment types not yet implemented.
            _ => {}
        }
    }

    let ctx = ctx?;

    Ok(Image {
        width: ctx.page_bitmap.width,
        height: ctx.page_bitmap.height,
        data: ctx.page_bitmap.data,
    })
}

/// Decoding context for a JBIG2 page.
///
/// This holds the page information and the page bitmap that regions are
/// decoded into.
pub(crate) struct DecodeContext {
    /// The parsed page information.
    pub page_info: PageInformation,
    /// The page bitmap that regions are combined into.
    pub page_bitmap: Bitmap,
}

/// Create a decode context from page information segment data.
///
/// This parses the page information and creates the initial page bitmap
/// with the default pixel value.
pub(crate) fn get_ctx(reader: &mut Reader<'_>) -> Result<DecodeContext, &'static str> {
    let page_info = parse_page_information(reader)?;

    // "Bit 2: Page default pixel value. This bit contains the initial value
    // for every pixel in the page, before any region segments are decoded
    // or drawn." (7.4.8.5)
    let mut page_bitmap = Bitmap::new(page_info.width, page_info.height);
    if page_info.flags.default_pixel != 0 {
        // Fill with true (black) if default pixel is 1.
        for pixel in &mut page_bitmap.data {
            *pixel = true;
        }
    }

    Ok(DecodeContext {
        page_info,
        page_bitmap,
    })
}
