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
mod dictionary;
mod file;
mod gray_scale;
mod huffman_table;
mod integer_decoder;
mod page_info;
mod reader;
mod region;
mod segment;

use crate::file::parse_segments_sequential;
use bitmap::DecodedRegion;
use dictionary::pattern::{PatternDictionary, decode_pattern_dictionary};
use dictionary::symbol::{SymbolDictionary, decode_symbol_dictionary};
use file::parse_file;
use huffman_table::HuffmanTable;
use page_info::{PageInformation, parse_page_information};
use reader::Reader;
use region::generic::decode_generic_region;
use region::generic_refinement::decode_generic_refinement_region;
use region::halftone::decode_halftone_region;
use region::text::decode_text_region;
use segment::SegmentType;

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

/// Decode a JBIG2 file from the given data.
///
/// The file is expected to use the sequential or random-access organization,
/// as defined in Annex D.1 and D.2.
pub fn decode(data: &[u8]) -> Result<Image, &'static str> {
    let file = parse_file(data)?;
    decode_with_segments(&file.segments)
}

/// Decode an embedded JBIG2 image. with the given global segments.
///
/// The file is expected to use the embedded organization defined in
/// Annex D.3.
pub fn decode_embedded(data: &[u8], globals: Option<&[u8]>) -> Result<Image, &'static str> {
    let mut segments = Vec::new();
    if let Some(globals_data) = globals {
        let mut reader = Reader::new(globals_data);
        parse_segments_sequential(&mut reader, &mut segments)?;
    };

    let mut reader = Reader::new(data);
    parse_segments_sequential(&mut reader, &mut segments)?;

    segments.sort_by_key(|seg| seg.header.segment_number);

    decode_with_segments(&segments)
}

fn decode_with_segments(segments: &[segment::Segment<'_>]) -> Result<Image, &'static str> {
    // Pre-scan for stripe height from EndOfStripe segments.
    let height_from_stripes = segments
        .iter()
        .filter(|seg| seg.header.segment_type == SegmentType::EndOfStripe)
        .filter_map(|seg| u32::from_be_bytes(seg.data.try_into().ok()?).checked_add(1))
        .max();

    // Find and parse page information segment first.
    let mut ctx = if let Some(page_info) = segments
        .iter()
        .find(|s| s.header.segment_type == SegmentType::PageInformation)
    {
        let mut reader = Reader::new(page_info.data);
        get_ctx(&mut reader, height_from_stripes)?
    } else {
        return Err("missing page information segment");
    };

    // Process all segments.
    for seg in segments {
        let mut reader = Reader::new(seg.data);

        match seg.header.segment_type {
            SegmentType::PageInformation => {
                // Already processed above, skip.
            }
            SegmentType::ImmediateGenericRegion | SegmentType::ImmediateLosslessGenericRegion => {
                let had_unknown_length = seg.header.data_length.is_none();
                let region = decode_generic_region(&mut reader, had_unknown_length)?;
                ctx.page_bitmap.combine(&region);
            }
            SegmentType::IntermediateGenericRegion => {
                // Intermediate segments cannot have unknown length.
                let region = decode_generic_region(&mut reader, false)?;
                ctx.store_region(seg.header.segment_number, region);
            }
            SegmentType::PatternDictionary => {
                let dictionary = decode_pattern_dictionary(&mut reader)?;
                ctx.store_pattern_dictionary(seg.header.segment_number, dictionary);
            }
            SegmentType::SymbolDictionary => {
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

                // Collect Huffman tables from referred table segments.
                let referred_tables: Vec<&HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_huffman_table(num))
                    .collect();

                let dictionary =
                    decode_symbol_dictionary(&mut reader, &input_symbols, &referred_tables)?;
                ctx.store_symbol_dictionary(seg.header.segment_number, dictionary);
            }
            SegmentType::ImmediateTextRegion | SegmentType::ImmediateLosslessTextRegion => {
                // Collect symbols from referred symbol dictionaries (SBSYMS).
                let symbols: Vec<&DecodedRegion> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_symbol_dictionary(num))
                    .flat_map(|dict| dict.exported_symbols.iter())
                    .collect();

                // Collect Huffman tables from referred table segments.
                // "These user-supplied Huffman decoding tables may be supplied either
                // as a Tables segment..." (7.4.3.1.6)
                let referred_tables: Vec<&HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_huffman_table(num))
                    .collect();

                let region = decode_text_region(&mut reader, &symbols, &referred_tables)?;
                ctx.page_bitmap.combine(&region);
            }
            SegmentType::IntermediateTextRegion => {
                // Collect symbols from referred symbol dictionaries (SBSYMS).
                let symbols: Vec<&DecodedRegion> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_symbol_dictionary(num))
                    .flat_map(|dict| dict.exported_symbols.iter())
                    .collect();

                // Collect Huffman tables from referred table segments.
                let referred_tables: Vec<&HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_huffman_table(num))
                    .collect();

                let region = decode_text_region(&mut reader, &symbols, &referred_tables)?;
                ctx.store_region(seg.header.segment_number, region);
            }
            SegmentType::ImmediateHalftoneRegion | SegmentType::ImmediateLosslessHalftoneRegion => {
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
            SegmentType::Tables => {
                // "Tables – see 7.4.13." (type 53)
                // "This segment contains data which defines one or more user-supplied
                // Huffman coding tables." (7.4.13)
                let table = HuffmanTable::read_custom(&mut reader)?;
                ctx.store_huffman_table(seg.header.segment_number, table);
            }
            SegmentType::EndOfPage | SegmentType::EndOfFile => {
                break;
            }
            // Other segment types not yet implemented.
            _ => {}
        }
    }

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
    pub(crate) _page_info: PageInformation,
    /// The page bitmap that regions are combined into.
    pub(crate) page_bitmap: DecodedRegion,
    /// Decoded intermediate regions, stored as (`segment_number`, region) pairs.
    pub(crate) referred_segments: Vec<(u32, DecodedRegion)>,
    /// Decoded pattern dictionaries, stored as (`segment_number`, dictionary) pairs.
    pub(crate) pattern_dictionaries: Vec<(u32, PatternDictionary)>,
    /// Decoded symbol dictionaries, stored as (`segment_number`, dictionary) pairs.
    pub(crate) symbol_dictionaries: Vec<(u32, SymbolDictionary)>,
    /// Decoded Huffman tables, stored as (`segment_number`, table) pairs.
    /// "Tables – see 7.4.13." (type 53)
    pub(crate) huffman_tables: Vec<(u32, HuffmanTable)>,
}

impl DecodeContext {
    /// Store a decoded region for later reference.
    fn store_region(&mut self, segment_number: u32, region: DecodedRegion) {
        self.referred_segments.push((segment_number, region));
    }

    /// Look up a referred segment by number.
    fn get_referred_segment(&self, segment_number: u32) -> Option<&DecodedRegion> {
        self.referred_segments
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.referred_segments[idx].1)
    }

    /// Store a decoded pattern dictionary for later reference.
    fn store_pattern_dictionary(&mut self, segment_number: u32, dictionary: PatternDictionary) {
        self.pattern_dictionaries.push((segment_number, dictionary));
    }

    /// Look up a pattern dictionary by segment number.
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

    /// Look up a symbol dictionary by segment number.
    fn get_symbol_dictionary(&self, segment_number: u32) -> Option<&SymbolDictionary> {
        self.symbol_dictionaries
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.symbol_dictionaries[idx].1)
    }

    /// Store a decoded Huffman table for later reference.
    fn store_huffman_table(&mut self, segment_number: u32, table: HuffmanTable) {
        self.huffman_tables.push((segment_number, table));
    }

    /// Look up a Huffman table by segment number.
    fn get_huffman_table(&self, segment_number: u32) -> Option<&HuffmanTable> {
        self.huffman_tables
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.huffman_tables[idx].1)
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
        _page_info: page_info,
        page_bitmap,
        referred_segments: Vec::new(),
        pattern_dictionaries: Vec::new(),
        symbol_dictionaries: Vec::new(),
        huffman_tables: Vec::new(),
    })
}
