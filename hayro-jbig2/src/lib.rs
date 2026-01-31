/*!
A memory-safe, pure-Rust JBIG2 decoder.

`hayro-jbig2` decodes JBIG2 images as specified in ITU-T T.88 (also known as
ISO/IEC 14492). JBIG2 is a bi-level image compression standard commonly used
in PDF documents for compressing scanned text documents.

The crate is `no_std` compatible but requires an allocator to be available.

# Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![allow(missing_docs)]

extern crate alloc;

use alloc::vec::Vec;

/// A decoder for JBIG2 images.
pub trait Decoder {
    /// Push a single pixel to the output.
    fn push_pixel(&mut self, black: bool);
    /// Push multiple chunks of 8 pixels of the same color.
    ///
    /// The `chunk_count` parameter indicates how many 8-pixel chunks to push.
    /// For example, if this method is called with `white = true` and
    /// `chunk_count = 10`, 80 white pixels are pushed (10 × 8 = 80).
    ///
    /// You can assume that this method is only called if the number of already
    /// pushed pixels is a multiple of 8 (i.e. byte-aligned).
    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32);
    /// Called when a row has been completed.
    fn next_line(&mut self);
}

mod arithmetic_decoder;
mod bitmap;
mod decode;
mod error;
mod file;
mod gray_scale;
mod huffman_table;
mod integer_decoder;
mod lazy;
mod page_info;
mod reader;
mod segment;
mod symbol_id_decoder;

use error::bail;
pub use error::{
    DecodeError, FormatError, HuffmanError, ParseError, RegionError, Result, SegmentError,
    SymbolError, TemplateError,
};

use crate::file::parse_segments_sequential;
use bitmap::Bitmap;
use decode::generic;
use decode::generic_refinement;
use decode::halftone;
use decode::pattern;
use decode::pattern::PatternDictionary;
use decode::symbol;
use decode::symbol::SymbolDictionary;
use decode::text;
use file::parse_file;
use huffman_table::{HuffmanTable, StandardHuffmanTables};
use page_info::{PageInformation, parse_page_information};
use reader::Reader;
use segment::SegmentType;

/// A decoded JBIG2 image.
#[derive(Debug, Clone)]
pub struct Image {
    /// The width of the image in pixels.
    pub width: u32,
    /// The height of the image in pixels.
    pub height: u32,
    /// Number of u32 words per row.
    stride: u32,
    /// The packed pixel data.
    data: Vec<u32>,
}

impl Image {
    /// Decode the image data into the decoder.
    pub fn decode<D: Decoder>(&self, decoder: &mut D) {
        let bytes_per_row = self.width.div_ceil(8) as usize;

        for row in self.data.chunks_exact(self.stride as usize) {
            let mut x = 0_u32;
            let mut chunk_byte: Option<u8> = None;
            let mut chunk_count = 0_u32;

            let bytes = row.iter().flat_map(|w| w.to_be_bytes()).take(bytes_per_row);

            for byte in bytes {
                let remaining = self.width - x;

                if remaining >= 8 && (byte == 0x00 || byte == 0xFF) {
                    // Continue the previous chunk.
                    if chunk_byte == Some(byte) {
                        chunk_count += 1;
                        x += 8;
                        continue;
                    }

                    // Flush previous chunk if any, then start new one.
                    if let Some(b) = chunk_byte {
                        decoder.push_pixel_chunk(b == 0xFF, chunk_count);
                    }

                    chunk_byte = Some(byte);
                    chunk_count = 1;
                    x += 8;

                    continue;
                }

                // Can't continue/start chunk, flush any existing chunk first.
                if let Some(b) = chunk_byte.take() {
                    decoder.push_pixel_chunk(b == 0xFF, chunk_count);
                    chunk_count = 0;
                }

                // Emit individual pixels.
                let count = remaining.min(8);
                for i in 0..count {
                    decoder.push_pixel((byte >> (7 - i)) & 1 != 0);
                }
                x += count;
            }

            // Flush any remaining chunk at end of row.
            if let Some(b) = chunk_byte {
                decoder.push_pixel_chunk(b == 0xFF, chunk_count);
            }

            decoder.next_line();
        }
    }
}

/// Decode a JBIG2 file from the given data.
///
/// The file is expected to use the sequential or random-access organization,
/// as defined in Annex D.1 and D.2.
pub fn decode(data: &[u8]) -> Result<Image> {
    let file = parse_file(data)?;
    decode_with_segments(&file.segments)
}

/// Decode an embedded JBIG2 image. with the given global segments.
///
/// The file is expected to use the embedded organization defined in
/// Annex D.3.
pub fn decode_embedded(data: &[u8], globals: Option<&[u8]>) -> Result<Image> {
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

fn decode_with_segments(segments: &[segment::Segment<'_>]) -> Result<Image> {
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
        bail!(FormatError::MissingPageInfo);
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
                let region = generic::decode(&mut reader, had_unknown_length)?;
                ctx.page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                );
            }
            SegmentType::IntermediateGenericRegion => {
                // Intermediate segments cannot have unknown length.
                let region = generic::decode(&mut reader, false)?;
                ctx.store_region(seg.header.segment_number, region.bitmap);
            }
            SegmentType::PatternDictionary => {
                let dictionary = pattern::decode(&mut reader)?;
                ctx.store_pattern_dictionary(seg.header.segment_number, dictionary);
            }
            SegmentType::SymbolDictionary => {
                // "1) Concatenate all the input symbol dictionaries to form SDINSYMS."
                // (6.5.5, step 1)
                // Collect references to avoid cloning; symbols are only cloned if re-exported.
                let input_symbols: Vec<&Bitmap> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_symbol_dictionary(num))
                    .flat_map(|dict| dict.exported_symbols.iter())
                    .collect();

                // Collect Huffman tables from referred table segments.
                let referred_tables: Vec<HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_huffman_table(num))
                    .cloned()
                    .collect();

                // Get retained contexts from the last referred symbol dictionary (7.4.2.2 step 3).
                let retained_contexts = seg
                    .header
                    .referred_to_segments
                    .last()
                    .and_then(|&num| ctx.get_symbol_dictionary(num))
                    .and_then(|dict| dict.retained_contexts.as_ref());

                let dictionary = symbol::decode(
                    &mut reader,
                    &input_symbols,
                    &referred_tables,
                    &ctx.standard_tables,
                    retained_contexts,
                )?;
                ctx.store_symbol_dictionary(seg.header.segment_number, dictionary);
            }
            SegmentType::ImmediateTextRegion | SegmentType::ImmediateLosslessTextRegion => {
                // Collect symbols from referred symbol dictionaries (SBSYMS).
                let symbols: Vec<&Bitmap> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_symbol_dictionary(num))
                    .flat_map(|dict| dict.exported_symbols.iter())
                    .collect();

                // Collect Huffman tables from referred table segments.
                // "These user-supplied Huffman decoding tables may be supplied either
                // as a Tables segment..." (7.4.3.1.6)
                let referred_tables: Vec<HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_huffman_table(num))
                    .cloned()
                    .collect();

                let region = text::decode(
                    &mut reader,
                    &symbols,
                    &referred_tables,
                    &ctx.standard_tables,
                )?;
                ctx.page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                );
            }
            SegmentType::IntermediateTextRegion => {
                // Collect symbols from referred symbol dictionaries (SBSYMS).
                let symbols: Vec<&Bitmap> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_symbol_dictionary(num))
                    .flat_map(|dict| dict.exported_symbols.iter())
                    .collect();

                // Collect Huffman tables from referred table segments.
                let referred_tables: Vec<HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| ctx.get_huffman_table(num))
                    .cloned()
                    .collect();

                let region = text::decode(
                    &mut reader,
                    &symbols,
                    &referred_tables,
                    &ctx.standard_tables,
                )?;
                ctx.store_region(seg.header.segment_number, region.bitmap);
            }
            SegmentType::ImmediateHalftoneRegion | SegmentType::ImmediateLosslessHalftoneRegion => {
                let pattern_dict = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| ctx.get_pattern_dictionary(num))
                    .ok_or(SegmentError::MissingPatternDictionary)?;

                let region = halftone::decode(&mut reader, pattern_dict)?;
                ctx.page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                );
            }
            SegmentType::IntermediateHalftoneRegion => {
                let pattern_dict = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| ctx.get_pattern_dictionary(num))
                    .ok_or(SegmentError::MissingPatternDictionary)?;

                let region = halftone::decode(&mut reader, pattern_dict)?;
                ctx.store_region(seg.header.segment_number, region.bitmap);
            }
            SegmentType::IntermediateGenericRefinementRegion => {
                // Same logic as immediate refinement, but store result instead of combining.
                let reference = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| ctx.get_referred_segment(num))
                    .unwrap_or(&ctx.page_bitmap);

                let region = generic_refinement::decode(&mut reader, reference)?;
                ctx.store_region(seg.header.segment_number, region.bitmap);
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

                let region = generic_refinement::decode(&mut reader, reference)?;
                ctx.page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                );
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
        stride: ctx.page_bitmap.stride,
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
    pub(crate) page_bitmap: Bitmap,
    /// Decoded intermediate regions, stored as (`segment_number`, region) pairs.
    pub(crate) referred_segments: Vec<(u32, Bitmap)>,
    /// Decoded pattern dictionaries, stored as (`segment_number`, dictionary) pairs.
    pub(crate) pattern_dictionaries: Vec<(u32, PatternDictionary)>,
    /// Decoded symbol dictionaries, stored as (`segment_number`, dictionary) pairs.
    pub(crate) symbol_dictionaries: Vec<(u32, SymbolDictionary)>,
    /// Decoded Huffman tables from table segments, stored as (`segment_number`, table) pairs.
    /// "Tables – see 7.4.13." (type 53)
    pub(crate) huffman_tables: Vec<(u32, HuffmanTable)>,
    /// Standard Huffman tables (`TABLE_A` through `TABLE_O`).
    pub(crate) standard_tables: StandardHuffmanTables,
}

impl DecodeContext {
    /// Store a decoded region for later reference.
    fn store_region(&mut self, segment_number: u32, region: Bitmap) {
        self.referred_segments.push((segment_number, region));
    }

    /// Look up a referred segment by number.
    fn get_referred_segment(&self, segment_number: u32) -> Option<&Bitmap> {
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
) -> Result<DecodeContext> {
    let page_info = parse_page_information(reader)?;

    // "A page's bitmap height may be declared in its page information segment
    // to be unknown (by specifying a height of 0xFFFFFFFF). In this case, the
    // page must be striped." (7.4.8.2)
    let height = if page_info.height == 0xFFFF_FFFF {
        height_from_stripes.ok_or(FormatError::UnknownPageHeight)?
    } else {
        page_info.height
    };

    // "Bit 2: Page default pixel value. This bit contains the initial value
    // for every pixel in the page, before any region segments are decoded
    // or drawn." (7.4.8.5)
    let page_bitmap = Bitmap::new_with(
        page_info.width,
        height,
        0,
        0,
        page_info.flags.default_pixel != 0,
    );

    Ok(DecodeContext {
        _page_info: page_info,
        page_bitmap,
        referred_segments: Vec::new(),
        pattern_dictionaries: Vec::new(),
        symbol_dictionaries: Vec::new(),
        huffman_tables: Vec::new(),
        standard_tables: StandardHuffmanTables::new(),
    })
}
