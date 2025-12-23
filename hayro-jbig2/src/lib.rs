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
mod huffman_table;
mod reader;
mod segment;

use bitmap::DecodedRegion;
use file::{File, parse_file};
use reader::Reader;
use segment::SegmentType;
use segment::generic_refinement_region::decode_generic_refinement_region;
use segment::generic_region::decode_generic_region;
use segment::halftone_region::decode_halftone_region;
use segment::page_info::{PageInformation, parse_page_information};
use segment::pattern_dictionary::{PatternDictionary, decode_pattern_dictionary};
use segment::symbol_dictionary::{SymbolDictionary, decode_symbol_dictionary};
use segment::text_region::decode_text_region;

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

    let height_from_stripes = scan_for_stripe_height(&file);

    let mut ctx: Result<DecodeContext, &'static str> = Err("attempted to decode\
    region before page information appeared");

    for seg in &file.segments {
        let mut reader = Reader::new(seg.data);

        match seg.header.segment_type {
            // "Page information – see 7.4.8." (type 48)
            SegmentType::PageInformation => {
                ctx = Ok(get_ctx(&mut reader, height_from_stripes)?);
            }
            SegmentType::ImmediateGenericRegion | SegmentType::ImmediateLosslessGenericRegion => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;
                let region = decode_generic_region(&mut reader)?;
                ctx.page_bitmap.combine(&region);
            }
            SegmentType::IntermediateGenericRegion => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;
                let region = decode_generic_region(&mut reader)?;
                ctx.store_region(seg.header.segment_number, region);
            }
            SegmentType::PatternDictionary => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;
                let dictionary = decode_pattern_dictionary(&mut reader)?;
                ctx.store_pattern_dictionary(seg.header.segment_number, dictionary);
            }
            SegmentType::SymbolDictionary => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;

                // "1) Concatenate all the input symbol dictionaries to form SDINSYMS."
                // (6.5.5, step 1)
                // Collect references to avoid cloning; symbols are only cloned if re-exported.
                let input_symbols: Vec<&DecodedRegion> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_symbol_dictionary(num))
                    .flat_map(|dict| dict.exported_symbols.iter())
                    .collect();

                let dictionary = decode_symbol_dictionary(&mut reader, &input_symbols)?;
                ctx.store_symbol_dictionary(seg.header.segment_number, dictionary);
            }
            SegmentType::ImmediateTextRegion | SegmentType::ImmediateLosslessTextRegion => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;

                // Collect symbols from referred symbol dictionaries (SBSYMS).
                let symbols: Vec<&DecodedRegion> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_symbol_dictionary(num))
                    .flat_map(|dict| dict.exported_symbols.iter())
                    .collect();

                let region = decode_text_region(&mut reader, &symbols)?;
                ctx.page_bitmap.combine(&region);
            }
            SegmentType::IntermediateTextRegion => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;

                // Collect symbols from referred symbol dictionaries (SBSYMS).
                let symbols: Vec<&DecodedRegion> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_symbol_dictionary(num))
                    .flat_map(|dict| dict.exported_symbols.iter())
                    .collect();

                let region = decode_text_region(&mut reader, &symbols)?;
                ctx.store_region(seg.header.segment_number, region);
            }
            SegmentType::ImmediateHalftoneRegion | SegmentType::ImmediateLosslessHalftoneRegion => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;

                let pattern_dict = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| ctx.get_pattern_dictionary(num))
                    .ok_or("halftone region requires a pattern dictionary")?;

                let region = decode_halftone_region(&mut reader, pattern_dict)?;
                ctx.page_bitmap.combine(&region);
            }
            SegmentType::IntermediateHalftoneRegion => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;

                let pattern_dict = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| ctx.get_pattern_dictionary(num))
                    .ok_or("halftone region requires a pattern dictionary")?;

                let region = decode_halftone_region(&mut reader, pattern_dict)?;
                ctx.store_region(seg.header.segment_number, region);
            }
            SegmentType::IntermediateGenericRefinementRegion => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;

                // Same logic as immediate refinement, but store result instead of combining.
                let reference = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| ctx.get_referred_segment(num))
                    .unwrap_or(&ctx.page_bitmap);

                let region = decode_generic_refinement_region(&mut reader, reference)?;
                ctx.store_region(seg.header.segment_number, region);
            }
            SegmentType::ImmediateGenericRefinementRegion
            | SegmentType::ImmediateLosslessGenericRefinementRegion => {
                let ctx = ctx.as_mut().map_err(|e| *e)?;

                // "3) Determine the buffer associated with the region segment that
                // this segment refers to." (7.4.7.5)
                //
                // "2) If there are no referred-to segments, then use the page
                // bitmap as the reference buffer." (7.4.7.5)
                let reference = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| ctx.get_referred_segment(num))
                    .unwrap_or(&ctx.page_bitmap);

                let region = decode_generic_refinement_region(&mut reader, reference)?;
                ctx.page_bitmap.combine(&region);
            }
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

/// Pre-scan segments to find the page height from EndOfStripe segments (7.4.10).
///
/// Returns the maximum Y coordinate + 1 from all EndOfStripe segments, or None
/// if no EndOfStripe segments are found.
fn scan_for_stripe_height(file: &File) -> Option<u32> {
    let mut max_y: Option<u32> = None;

    for seg in &file.segments {
        if seg.header.segment_type == SegmentType::EndOfStripe {
            let height = u32::from_be_bytes(seg.data.try_into().ok()?).checked_add(1)?;
            max_y = Some(max_y.map_or(height, |m| m.max(height)));
        }
    }

    max_y
}

/// Decoding context for a JBIG2 page.
///
/// This holds the page information and the page bitmap that regions are
/// decoded into.
pub(crate) struct DecodeContext {
    /// The parsed page information.
    pub page_info: PageInformation,
    /// The page bitmap that regions are combined into.
    pub page_bitmap: DecodedRegion,
    /// Decoded intermediate regions, stored as (segment_number, region) pairs.
    pub referred_segments: Vec<(u32, DecodedRegion)>,
    /// Decoded pattern dictionaries, stored as (segment_number, dictionary) pairs.
    pub pattern_dictionaries: Vec<(u32, PatternDictionary)>,
    /// Decoded symbol dictionaries, stored as (segment_number, dictionary) pairs.
    pub symbol_dictionaries: Vec<(u32, SymbolDictionary)>,
}

impl DecodeContext {
    /// Store a decoded region for later reference.
    fn store_region(&mut self, segment_number: u32, region: DecodedRegion) {
        self.referred_segments.push((segment_number, region));
    }

    /// Look up a referred segment by number using binary search.
    fn get_referred_segment(&self, segment_number: u32) -> Option<&DecodedRegion> {
        self.referred_segments
            // We iterate over the segments in order (which themselves are sorted),
            // so here we can just do a binary search.
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.referred_segments[idx].1)
    }

    /// Store a decoded pattern dictionary for later reference.
    fn store_pattern_dictionary(&mut self, segment_number: u32, dictionary: PatternDictionary) {
        self.pattern_dictionaries.push((segment_number, dictionary));
    }

    /// Look up a pattern dictionary by segment number using binary search.
    fn get_pattern_dictionary(&self, segment_number: u32) -> Option<&PatternDictionary> {
        self.pattern_dictionaries
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.pattern_dictionaries[idx].1)
    }

    /// Store a decoded symbol dictionary for later reference.
    fn store_symbol_dictionary(&mut self, segment_number: u32, dictionary: SymbolDictionary) {
        self.symbol_dictionaries.push((segment_number, dictionary));
    }

    /// Look up a symbol dictionary by segment number using binary search.
    fn get_symbol_dictionary(&self, segment_number: u32) -> Option<&SymbolDictionary> {
        self.symbol_dictionaries
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.symbol_dictionaries[idx].1)
    }
}

/// Create a decode context from page information segment data.
///
/// This parses the page information and creates the initial page bitmap
/// with the default pixel value.
pub(crate) fn get_ctx(
    reader: &mut Reader<'_>,
    height_from_stripes: Option<u32>,
) -> Result<DecodeContext, &'static str> {
    let page_info = parse_page_information(reader)?;

    // "A page's bitmap height may be declared in its page information segment
    // to be unknown (by specifying a height of 0xFFFFFFFF). In this case, the
    // page must be striped." (7.4.8.2)
    let height = if page_info.height == 0xFFFF_FFFF {
        height_from_stripes.ok_or("page height is missing")?
    } else {
        page_info.height
    };

    // "Bit 2: Page default pixel value. This bit contains the initial value
    // for every pixel in the page, before any region segments are decoded
    // or drawn." (7.4.8.5)
    let mut page_bitmap = DecodedRegion::new(page_info.width, height);
    if page_info.flags.default_pixel != 0 {
        // Fill with true (black) if default pixel is 1.
        for pixel in &mut page_bitmap.data {
            *pixel = true;
        }
    }

    Ok(DecodeContext {
        page_info,
        page_bitmap,
        referred_segments: Vec::new(),
        pattern_dictionaries: Vec::new(),
        symbol_dictionaries: Vec::new(),
    })
}
