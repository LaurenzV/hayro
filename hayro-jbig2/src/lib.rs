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
use core::ops::ControlFlow;

use crate::arithmetic_decoder::ArithmeticDecoderContext;

/// A reusable context for decoding JBIG2 images.
#[derive(Default)]
pub struct DecoderContext {
    pub(crate) page_state: PageState,
    pub(crate) scratch_buffers: ScratchBuffers,
    pub(crate) page_bitmap: Bitmap,
}

#[derive(Default)]
pub(crate) struct ScratchBuffers {
    pub(crate) contexts: Vec<ArithmeticDecoderContext>,
}

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

#[cfg(feature = "image")]
pub mod integration;

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
mod simd;
mod symbol_id_decoder;

use error::bail;
pub use error::{
    DecodeError, FormatError, HuffmanError, OverflowError, ParseError, RegionError, Result,
    SegmentError, SymbolError, TemplateError,
};

use crate::file::parse_segments_sequential;
use bitmap::Bitmap;
use decode::CombinationOperator;
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

/// Settings for decoding JBIG2 images.
#[derive(Debug, Clone, Copy, Default)]
pub struct DecodeSettings {
    /// Whether to reject data that violates the specification, instead of
    /// decoding it as well as possible.
    ///
    /// Defaults to `false`.
    pub strict: bool,
}

/// A JBIG2 image.
pub struct Image<'a> {
    /// The parsed segments.
    segments: Vec<segment::Segment<'a>>,
    /// The width of the image in pixels.
    width: u32,
    /// The height of the image in pixels.
    height: u32,
    /// The height determined from `EndOfStripe` segments, if applicable.
    height_from_stripes: Option<u32>,
    /// Whether strict mode is enabled.
    strict: bool,
}

impl<'a> Image<'a> {
    /// Parse a JBIG2 file from the given data.
    ///
    /// The file is expected to use the sequential or random-access organization,
    /// as defined in Annex D.1 and D.2.
    pub fn new(data: &'a [u8], settings: &DecodeSettings) -> Result<Self> {
        let file = parse_file(data, settings.strict)?;
        Self::from_segments(file.segments, settings.strict)
    }

    /// Parse an embedded JBIG2 image with optional global segments.
    ///
    /// The file is expected to use the embedded organization defined in
    /// Annex D.3.
    pub fn new_embedded(
        data: &'a [u8],
        globals: Option<&'a [u8]>,
        settings: &DecodeSettings,
    ) -> Result<Self> {
        let mut segments = Vec::new();
        if let Some(globals_data) = globals {
            let mut reader = Reader::new(globals_data);
            parse_segments_sequential(&mut reader, &mut segments, settings.strict)?;
        };

        let mut reader = Reader::new(data);
        parse_segments_sequential(&mut reader, &mut segments, settings.strict)?;

        segments.sort_by_key(|seg| seg.header.segment_number);

        Self::from_segments(segments, settings.strict)
    }

    fn from_segments(segments: Vec<segment::Segment<'a>>, strict: bool) -> Result<Self> {
        // Pre-scan for stripe height from EndOfStripe segments.
        let height_from_stripes = segments
            .iter()
            .filter(|seg| seg.header.segment_type == SegmentType::EndOfStripe)
            .filter_map(|seg| u32::from_be_bytes(seg.data.try_into().ok()?).checked_add(1))
            .max();

        // Find and parse page information to extract dimensions.
        let page_info_seg = segments
            .iter()
            .find(|s| s.header.segment_type == SegmentType::PageInformation)
            .ok_or(FormatError::MissingPageInfo)?;

        let mut reader = Reader::new(page_info_seg.data);
        let page_info = parse_page_information(&mut reader)?;

        // "A page's bitmap height may be declared in its page information segment
        // to be unknown (by specifying a height of 0xFFFFFFFF). In this case, the
        // page must be striped." (7.4.8.2)
        let height = if page_info.height == 0xFFFF_FFFF {
            height_from_stripes.ok_or(FormatError::UnknownPageHeight)?
        } else {
            page_info.height
        };

        if page_info.width == 0 || height == 0 {
            bail!(FormatError::EmptyPage);
        }

        Ok(Self {
            segments,
            width: page_info.width,
            height,
            height_from_stripes,
            strict,
        })
    }

    /// The width of the image in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height of the image in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Decode the image data through the given [`Decoder`].
    pub fn decode<D: Decoder>(&self, decoder: &mut D) -> Result<()> {
        let mut ctx = DecoderContext::default();

        self.decode_with(decoder, &mut ctx)
    }

    /// Decode the image data through the given [`Decoder`] and [`DecoderContext`].
    ///
    /// This is useful in case you want to convert multiple JBIG2 images,
    /// as it allows `hayro-jbig2` to reuse allocations during decoding.
    pub fn decode_with<D: Decoder>(&self, decoder: &mut D, ctx: &mut DecoderContext) -> Result<()> {
        decode_segments(&self.segments, self.height_from_stripes, self.strict, ctx)?;
        emit_bitmap(&ctx.page_bitmap, decoder);

        Ok(())
    }
}

fn emit_bitmap<D: Decoder>(bitmap: &Bitmap, decoder: &mut D) {
    let width = bitmap.width;
    let bytes_per_row = width.div_ceil(8) as usize;

    for row in bitmap.data.chunks_exact(bitmap.stride as usize) {
        let mut x = 0_u32;
        let mut chunk_byte: Option<u8> = None;
        let mut chunk_count = 0_u32;

        let bytes = row.iter().flat_map(|w| w.to_be_bytes()).take(bytes_per_row);

        for byte in bytes {
            let remaining = width - x;

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

fn decode_segments(
    segments: &[segment::Segment<'_>],
    height_from_stripes: Option<u32>,
    strict: bool,
    decoder_ctx: &mut DecoderContext,
) -> Result<()> {
    // Find and parse page information segment first.
    if let Some(page_info) = segments
        .iter()
        .find(|s| s.header.segment_type == SegmentType::PageInformation)
    {
        let mut reader = Reader::new(page_info.data);
        init_page(
            &mut reader,
            height_from_stripes,
            &mut decoder_ctx.page_state,
            &mut decoder_ctx.page_bitmap,
        )?;
    } else {
        bail!(FormatError::MissingPageInfo);
    }

    let page_bitmap = &mut decoder_ctx.page_bitmap;
    let page_state = &mut decoder_ctx.page_state;
    let scratch_buffers = &mut decoder_ctx.scratch_buffers;

    for seg in segments {
        match decode_segment(seg, page_bitmap, page_state, scratch_buffers, strict) {
            Ok(ControlFlow::Continue(())) => {}
            Ok(ControlFlow::Break(())) => break,
            // No other segment can refer to an immediate region, so the rest of
            // the page doesn't depend on it.
            Err(_) if !strict && seg.header.segment_type.is_immediate_region() => {
                // It might have been decoded directly into the page.
                page_state.page_pristine = false;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

fn decode_segment(
    seg: &segment::Segment<'_>,
    page_bitmap: &mut Bitmap,
    page_state: &mut PageState,
    scratch_buffers: &mut ScratchBuffers,
    strict: bool,
) -> Result<ControlFlow<()>> {
    let mut reader = Reader::new(seg.data);

    match seg.header.segment_type {
        SegmentType::PageInformation => {
            // Already processed above, skip.
        }
        SegmentType::ImmediateGenericRegion | SegmentType::ImmediateLosslessGenericRegion => {
            let had_unknown_length = seg.header.data_length.is_none();
            let header = generic::parse(&mut reader, had_unknown_length, strict)?;

            if page_state.can_decode_directly(page_bitmap, &header.region_info, false) {
                generic::decode_into(&header, page_bitmap, scratch_buffers)?;
            } else {
                let region = generic::decode(&header, scratch_buffers)?;
                page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                );
            }
            page_state.page_pristine = false;
        }
        SegmentType::IntermediateGenericRegion => {
            // Intermediate segments cannot have unknown length.
            let header = generic::parse(&mut reader, false, strict)?;
            let region = generic::decode(&header, scratch_buffers)?;
            page_state.store_region(seg.header.segment_number, region.bitmap);
        }
        SegmentType::PatternDictionary => {
            let header = pattern::parse(&mut reader, strict)?;
            let dictionary = pattern::decode(&header, scratch_buffers)?;
            page_state.store_pattern_dictionary(seg.header.segment_number, dictionary);
        }
        SegmentType::SymbolDictionary => {
            // "1) Concatenate all the input symbol dictionaries to form SDINSYMS."
            // (6.5.5, step 1)
            let (input_symbols, referred_tables) =
                page_state.get_symbols_and_tables(&seg.header.referred_to_segments, strict)?;

            // Get retained contexts from the last referred symbol dictionary (7.4.2.2 step 3).
            let retained_contexts = seg
                .header
                .referred_to_segments
                .iter()
                .rev()
                .find_map(|&num| page_state.get_symbol_dictionary(num))
                .and_then(|dict| dict.retained_contexts.as_ref());

            let header = symbol::parse(&mut reader, strict)?;
            let dictionary = symbol::decode(
                &header,
                &input_symbols,
                &referred_tables,
                &page_state.standard_tables,
                retained_contexts,
            )?;
            page_state.store_symbol_dictionary(seg.header.segment_number, dictionary);
        }
        SegmentType::ImmediateTextRegion | SegmentType::ImmediateLosslessTextRegion => {
            let (symbols, referred_tables) =
                page_state.get_symbols_and_tables(&seg.header.referred_to_segments, strict)?;

            let header = text::parse(&mut reader, symbols.len() as u32, strict)?;

            if page_state.can_decode_directly(
                page_bitmap,
                &header.region_info,
                header.flags.default_pixel,
            ) {
                text::decode_into(
                    &header,
                    &symbols,
                    &referred_tables,
                    &page_state.standard_tables,
                    page_bitmap,
                    scratch_buffers,
                )?;
            } else {
                let region = text::decode(
                    &header,
                    &symbols,
                    &referred_tables,
                    &page_state.standard_tables,
                    scratch_buffers,
                )?;
                page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                );
            }
            page_state.page_pristine = false;
        }
        SegmentType::IntermediateTextRegion => {
            let (symbols, referred_tables) =
                page_state.get_symbols_and_tables(&seg.header.referred_to_segments, strict)?;

            let header = text::parse(&mut reader, symbols.len() as u32, strict)?;
            let region = text::decode(
                &header,
                &symbols,
                &referred_tables,
                &page_state.standard_tables,
                scratch_buffers,
            )?;
            page_state.store_region(seg.header.segment_number, region.bitmap);
        }
        SegmentType::ImmediateHalftoneRegion | SegmentType::ImmediateLosslessHalftoneRegion => {
            let pattern_dict = seg
                .header
                .referred_to_segments
                .first()
                .and_then(|&num| page_state.get_pattern_dictionary(num))
                .ok_or(SegmentError::MissingPatternDictionary)?;

            let header = halftone::parse(&mut reader, strict)?;

            if page_state.can_decode_directly(
                page_bitmap,
                &header.region_info,
                header.flags.initial_pixel_color,
            ) {
                halftone::decode_into(&header, pattern_dict, page_bitmap, scratch_buffers)?;
            } else {
                let region = halftone::decode(&header, pattern_dict, scratch_buffers)?;
                page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                );
            }
            page_state.page_pristine = false;
        }
        SegmentType::IntermediateHalftoneRegion => {
            let pattern_dict = seg
                .header
                .referred_to_segments
                .first()
                .and_then(|&num| page_state.get_pattern_dictionary(num))
                .ok_or(SegmentError::MissingPatternDictionary)?;

            let header = halftone::parse(&mut reader, strict)?;
            let region = halftone::decode(&header, pattern_dict, scratch_buffers)?;
            page_state.store_region(seg.header.segment_number, region.bitmap);
        }
        SegmentType::IntermediateGenericRefinementRegion => {
            // Same logic as immediate refinement, but store result instead of combining.
            let referred_segment =
                page_state.get_refined_region(&seg.header.referred_to_segments, strict)?;

            if strict && referred_segment.is_none() {
                bail!(SegmentError::InvalidReference);
            }

            let header = generic_refinement::parse(&mut reader)?;

            let page_area;
            let reference = match referred_segment {
                Some(referred_segment) => referred_segment,
                None => {
                    page_area = page_bitmap_area(page_bitmap, &header.region_info)?;
                    &page_area
                }
            };
            let region = generic_refinement::decode(&header, reference, scratch_buffers)?;

            page_state.store_region(seg.header.segment_number, region.bitmap);
        }
        SegmentType::ImmediateGenericRefinementRegion
        | SegmentType::ImmediateLosslessGenericRefinementRegion => {
            // "3) Determine the buffer associated with the region segment that
            // this segment refers to." (7.4.7.5)
            //
            // "If this segment does not refer to another region segment, set
            // GRREFERENCE to be a bitmap containing the current contents of the
            // page buffer (see clause 8), restricted to the area of the page
            // buffer specified by this segment's region segment information
            // field." (7.4.7.4)
            let referred_segment =
                page_state.get_refined_region(&seg.header.referred_to_segments, strict)?;

            let header = generic_refinement::parse(&mut reader)?;

            if let Some(referred_segment) = referred_segment
                && page_state.can_decode_directly(page_bitmap, &header.region_info, false)
            {
                generic_refinement::decode_into(
                    &header,
                    referred_segment,
                    page_bitmap,
                    scratch_buffers,
                )?;
            } else {
                let page_area;
                let reference = match referred_segment {
                    Some(referred_segment) => referred_segment,
                    None => {
                        page_area = page_bitmap_area(page_bitmap, &header.region_info)?;
                        &page_area
                    }
                };
                let region = generic_refinement::decode(&header, reference, scratch_buffers)?;
                page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                );
            }

            page_state.page_pristine = false;
        }
        SegmentType::Tables => {
            // "Tables – see 7.4.13." (type 53)
            // "This segment contains data which defines one or more user-supplied
            // Huffman coding tables." (7.4.13)
            let table = HuffmanTable::read_custom(&mut reader)?;
            page_state.store_huffman_table(seg.header.segment_number, table);
        }
        SegmentType::EndOfPage | SegmentType::EndOfFile => {
            return Ok(ControlFlow::Break(()));
        }
        SegmentType::Extension if strict => {
            let extension_type = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
            if extension_type & 0x8000_0000 != 0 {
                bail!(DecodeError::Unsupported);
            }
        }
        // Other segment types not yet implemented.
        _ => {}
    }

    Ok(ControlFlow::Continue(()))
}

/// Page-level decoding state for a JBIG2 page.
#[derive(Default)]
pub(crate) struct PageState {
    /// The parsed page information.
    pub(crate) page_info: PageInformation,
    /// Whether the page bitmap is still in its initial state (not yet painted to).
    pub(crate) page_pristine: bool,
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

impl PageState {
    fn reset(&mut self, page_info: PageInformation) {
        self.page_info = page_info;
        self.page_pristine = true;
        self.referred_segments.clear();
        self.pattern_dictionaries.clear();
        self.symbol_dictionaries.clear();
        self.huffman_tables.clear();
        // Standard tables are lazily built and reused across images.
    }

    /// Check if an immediate region can be decoded directly into the page bitmap.
    fn can_decode_directly(
        &self,
        page_bitmap: &Bitmap,
        region_info: &decode::RegionSegmentInfo,
        region_default_pixel: bool,
    ) -> bool {
        if !self.page_pristine {
            return false;
        }

        let covers_page = region_info.x_location == 0
            && region_info.y_location == 0
            && region_info.width == page_bitmap.width
            && region_info.height == page_bitmap.height;

        if !covers_page {
            return false;
        }

        let page_default_is_zero = self.page_info.flags.default_pixel == 0;

        if region_default_pixel == page_default_is_zero {
            return false;
        }

        let op = region_info.combination_operator;
        match op {
            CombinationOperator::Replace => true,
            CombinationOperator::Or | CombinationOperator::Xor => page_default_is_zero,
            CombinationOperator::And | CombinationOperator::Xnor => !page_default_is_zero,
        }
    }

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

    fn get_refined_region(
        &self,
        referred_segments: &[u32],
        strict: bool,
    ) -> Result<Option<&Bitmap>> {
        let Some(&num) = referred_segments.first() else {
            return Ok(None);
        };

        match self.get_referred_segment(num) {
            None if strict => bail!(SegmentError::InvalidReference),
            region => Ok(region),
        }
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

    fn get_symbols_and_tables(
        &self,
        referred_segments: &[u32],
        strict: bool,
    ) -> Result<(Vec<&Bitmap>, Vec<HuffmanTable>)> {
        let mut symbols = Vec::new();
        let mut tables = Vec::new();

        for &num in referred_segments {
            if let Some(dictionary) = self.get_symbol_dictionary(num) {
                symbols.extend(dictionary.exported_symbols.iter());
            } else if let Some(table) = self.get_huffman_table(num) {
                tables.push(table.clone());
            } else if strict {
                bail!(SegmentError::InvalidReference);
            }
        }

        Ok((symbols, tables))
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

fn page_bitmap_area(page: &Bitmap, region_info: &decode::RegionSegmentInfo) -> Result<Bitmap> {
    let mut area = Bitmap::new_with(
        region_info.width,
        region_info.height,
        region_info.x_location,
        region_info.y_location,
        false,
    )?;
    area.combine(
        page,
        -i32::try_from(region_info.x_location).unwrap_or(i32::MAX),
        -i32::try_from(region_info.y_location).unwrap_or(i32::MAX),
        CombinationOperator::Replace,
    );

    Ok(area)
}

/// Parse page information and initialize the page bitmap.
fn init_page(
    reader: &mut Reader<'_>,
    height_from_stripes: Option<u32>,
    page: &mut PageState,
    bitmap: &mut Bitmap,
) -> Result<()> {
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
    bitmap.reinitialize(page_info.width, height, page_info.flags.default_pixel != 0)?;

    page.reset(page_info);

    Ok(())
}
