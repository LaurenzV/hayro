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

use alloc::collections::BTreeMap;
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
mod sync;

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
use decode::symbol::{MAX_SYMBOLS, SymbolDictionary};
use decode::text;
use file::parse_file;
use huffman_table::{HuffmanTable, StandardHuffmanTables};
use page_info::{PageInformation, parse_page_information};
use reader::Reader;
use segment::{Segment, SegmentType};
use sync::OnceLock;

/// Settings for decoding JBIG2 images.
#[derive(Debug, Clone, Copy, Default)]
pub struct DecodeSettings {
    /// Whether to reject data that violates the specification, instead of
    /// decoding it as well as possible.
    ///
    /// Defaults to `false`.
    pub strict: bool,
}

/// A JBIG2 file in the sequential or random-access organization (Annex D.1, D.2).
pub struct File<'a> {
    pages: Vec<FilePage<'a>>,
    globals: Option<LazyGlobals<'a>>,
    strict: bool,
}

struct FilePage<'a> {
    number: u32,
    segments: Vec<Segment<'a>>,
    size: PageSize,
}

/// The global segments of a [`File`], decoded on first use.
struct LazyGlobals<'a> {
    segments: Vec<Segment<'a>>,
    decoded: OnceLock<Result<Globals>>,
}

impl LazyGlobals<'_> {
    fn get(&self, strict: bool) -> Result<&Globals> {
        self.decoded
            .get_or_init(|| Globals::from_segments(&self.segments, strict))
            .as_ref()
            .map_err(|e| *e)
    }
}

impl<'a> File<'a> {
    /// Parse a JBIG2 file.
    pub fn new(data: &'a [u8], settings: &DecodeSettings) -> Result<Self> {
        let strict = settings.strict;
        let file = parse_file(data, strict)?;

        let mut globals = Vec::new();
        let mut pages: Vec<FilePage<'a>> = Vec::new();
        let mut page_indices = BTreeMap::new();

        for seg in file.segments {
            let number = seg.header.page_association;
            if number == 0 {
                if strict && seg.header.segment_type.requires_page() {
                    bail!(SegmentError::MissingPageAssociation);
                }

                globals.push(seg);
                continue;
            }

            let index = *page_indices.entry(number).or_insert_with(|| {
                pages.push(FilePage {
                    number,
                    segments: Vec::new(),
                    size: PageSize::default(),
                });
                pages.len() - 1
            });
            pages[index].segments.push(seg);
        }

        if strict
            && file
                .header
                .number_of_pages
                .is_some_and(|n| n as usize != pages.len())
        {
            bail!(FormatError::PageCountMismatch);
        }

        for page in &mut pages {
            if strict {
                let end_of_page_count = page
                    .segments
                    .iter()
                    .filter(|seg| seg.header.segment_type == SegmentType::EndOfPage)
                    .count();

                if end_of_page_count != 1 {
                    bail!(FormatError::InvalidEndOfPage);
                }
            }

            page.size = PageSize::new(&page.segments)?;
        }

        Ok(Self {
            pages,
            globals: (!globals.is_empty()).then(|| LazyGlobals {
                segments: globals,
                decoded: OnceLock::new(),
            }),
            strict,
        })
    }

    /// The number of pages in the file.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The page at `index`, in file order.
    pub fn page(&self, index: usize) -> Option<Page<'_>> {
        let page = self.pages.get(index)?;

        Some(Page {
            number: page.number,
            segments: PageSegments::Borrowed(&page.segments),
            globals: self.globals.as_ref().map(GlobalsRef::Lazy),
            size: page.size,
            strict: self.strict,
        })
    }

    /// The pages of the file, in file order.
    pub fn pages(&self) -> impl Iterator<Item = Page<'_>> {
        (0..self.page_count()).filter_map(|index| self.page(index))
    }
}

/// Decoded global segments that embedded images can share, such as a PDF
/// `JBIG2Globals` stream.
pub struct Globals {
    dictionaries: Dictionaries,
}

impl Globals {
    /// Parse and decode global segments in the embedded organization (Annex D.3).
    pub fn new(data: &[u8], settings: &DecodeSettings) -> Result<Self> {
        let mut segments = Vec::new();
        let mut reader = Reader::new(data);
        parse_segments_sequential(&mut reader, &mut segments, settings.strict)?;
        segments.sort_by_key(|seg| seg.header.segment_number);

        Self::from_segments(&segments, settings.strict)
    }

    fn from_segments(segments: &[Segment<'_>], strict: bool) -> Result<Self> {
        let mut dictionaries = Dictionaries::default();
        let standard_tables = StandardHuffmanTables::default();
        let mut scratch_buffers = ScratchBuffers::default();

        for seg in segments {
            decode_dictionary_segment(
                seg,
                &mut dictionaries,
                None,
                &standard_tables,
                &mut scratch_buffers,
                strict,
            )?;
        }

        Ok(Self { dictionaries })
    }
}

/// A JBIG2 page, from a [`File`] or embedded in another format.
pub struct Page<'a> {
    number: u32,
    segments: PageSegments<'a>,
    globals: Option<GlobalsRef<'a>>,
    size: PageSize,
    strict: bool,
}

enum PageSegments<'a> {
    Owned(Vec<Segment<'a>>),
    Borrowed(&'a [Segment<'a>]),
}

enum GlobalsRef<'a> {
    Decoded(&'a Globals),
    Lazy(&'a LazyGlobals<'a>),
}

impl<'a> Page<'a> {
    /// Parse a page in the embedded organization (Annex D.3), with optional globals.
    pub fn new_embedded(
        data: &'a [u8],
        globals: Option<&'a Globals>,
        settings: &DecodeSettings,
    ) -> Result<Self> {
        let mut segments = Vec::new();
        let mut reader = Reader::new(data);
        parse_segments_sequential(&mut reader, &mut segments, settings.strict)?;

        segments.sort_by_key(|seg| seg.header.segment_number);

        let size = PageSize::new(&segments)?;
        let number = segments
            .iter()
            .find(|seg| seg.header.segment_type == SegmentType::PageInformation)
            .map_or(1, |seg| seg.header.page_association);

        Ok(Self {
            number,
            segments: PageSegments::Owned(segments),
            globals: globals.map(GlobalsRef::Decoded),
            size,
            strict: settings.strict,
        })
    }

    /// The page number from the page's segment headers.
    pub fn number(&self) -> u32 {
        self.number
    }

    /// The width of the page in pixels.
    pub fn width(&self) -> u32 {
        self.size.width
    }

    /// The height of the page in pixels.
    pub fn height(&self) -> u32 {
        self.size.height
    }

    /// Decode the page through the given [`Decoder`].
    pub fn decode<D: Decoder>(&self, decoder: &mut D) -> Result<()> {
        let mut ctx = DecoderContext::default();

        self.decode_with(decoder, &mut ctx)
    }

    /// Like [`Page::decode`], but reuses the allocations in `ctx` across pages.
    pub fn decode_with<D: Decoder>(&self, decoder: &mut D, ctx: &mut DecoderContext) -> Result<()> {
        let segments = match &self.segments {
            PageSegments::Owned(segments) => segments,
            PageSegments::Borrowed(segments) => *segments,
        };
        let globals = match self.globals {
            Some(GlobalsRef::Decoded(globals)) => Some(globals),
            Some(GlobalsRef::Lazy(globals)) => Some(globals.get(self.strict)?),
            None => None,
        };

        decode_segments(
            segments,
            globals,
            self.size.height_from_stripes,
            self.strict,
            ctx,
        )?;
        emit_bitmap(&ctx.page_bitmap, decoder);

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PageSize {
    width: u32,
    height: u32,
    /// The height determined from `EndOfStripe` segments, if applicable.
    height_from_stripes: Option<u32>,
}

impl PageSize {
    fn new(segments: &[Segment<'_>]) -> Result<Self> {
        let height_from_stripes = segments
            .iter()
            .filter(|seg| seg.header.segment_type == SegmentType::EndOfStripe)
            .filter_map(|seg| u32::from_be_bytes(seg.data.try_into().ok()?).checked_add(1))
            .max();

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
            width: page_info.width,
            height,
            height_from_stripes,
        })
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
    segments: &[Segment<'_>],
    globals: Option<&Globals>,
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
        match decode_segment(
            seg,
            page_bitmap,
            page_state,
            globals,
            scratch_buffers,
            strict,
        ) {
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
    seg: &Segment<'_>,
    page_bitmap: &mut Bitmap,
    page_state: &mut PageState,
    globals: Option<&Globals>,
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
        SegmentType::PatternDictionary
        | SegmentType::SymbolDictionary
        | SegmentType::Tables
        | SegmentType::Extension => decode_dictionary_segment(
            seg,
            &mut page_state.dictionaries,
            globals,
            &page_state.standard_tables,
            scratch_buffers,
            strict,
        )?,
        SegmentType::ImmediateTextRegion | SegmentType::ImmediateLosslessTextRegion => {
            let (symbols, referred_tables) = page_state.dictionaries.get_symbols_and_tables(
                &seg.header.referred_to_segments,
                globals,
                strict,
            )?;

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
            let (symbols, referred_tables) = page_state.dictionaries.get_symbols_and_tables(
                &seg.header.referred_to_segments,
                globals,
                strict,
            )?;

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
                .and_then(|&num| page_state.dictionaries.get_pattern_dictionary(num, globals))
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
                .and_then(|&num| page_state.dictionaries.get_pattern_dictionary(num, globals))
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
        SegmentType::EndOfPage | SegmentType::EndOfFile => {
            return Ok(ControlFlow::Break(()));
        }
        // Other segment types not yet implemented.
        _ => {}
    }

    Ok(ControlFlow::Continue(()))
}

fn decode_dictionary_segment(
    seg: &Segment<'_>,
    dictionaries: &mut Dictionaries,
    globals: Option<&Globals>,
    standard_tables: &StandardHuffmanTables,
    scratch_buffers: &mut ScratchBuffers,
    strict: bool,
) -> Result<()> {
    let mut reader = Reader::new(seg.data);

    match seg.header.segment_type {
        SegmentType::PatternDictionary => {
            let header = pattern::parse(&mut reader, strict)?;
            let dictionary = pattern::decode(&header, scratch_buffers)?;
            dictionaries.store_pattern_dictionary(seg.header.segment_number, dictionary);
        }
        SegmentType::SymbolDictionary => {
            // "1) Concatenate all the input symbol dictionaries to form SDINSYMS."
            // (6.5.5, step 1)
            let (input_symbols, referred_tables) = dictionaries.get_symbols_and_tables(
                &seg.header.referred_to_segments,
                globals,
                strict,
            )?;

            // Get retained contexts from the last referred symbol dictionary (7.4.2.2 step 3).
            let retained_contexts = seg
                .header
                .referred_to_segments
                .iter()
                .rev()
                .find_map(|&num| dictionaries.get_symbol_dictionary(num, globals))
                .and_then(|dict| dict.retained_contexts.as_ref());

            let header = symbol::parse(&mut reader, strict)?;
            let dictionary = symbol::decode(
                &header,
                &input_symbols,
                &referred_tables,
                standard_tables,
                retained_contexts,
            )?;
            dictionaries.store_symbol_dictionary(seg.header.segment_number, dictionary);
        }
        SegmentType::Tables => {
            // "Tables – see 7.4.13." (type 53)
            // "This segment contains data which defines one or more user-supplied
            // Huffman coding tables." (7.4.13)
            let table = HuffmanTable::read_custom(&mut reader)?;
            dictionaries.store_huffman_table(seg.header.segment_number, table);
        }
        SegmentType::Extension if strict => {
            let extension_type = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
            if extension_type & 0x8000_0000 != 0 {
                bail!(DecodeError::Unsupported);
            }
        }
        _ => {}
    }

    Ok(())
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
    /// The dictionaries and tables of the page.
    pub(crate) dictionaries: Dictionaries,
    /// Standard Huffman tables (`TABLE_A` through `TABLE_O`).
    pub(crate) standard_tables: StandardHuffmanTables,
}

impl PageState {
    fn reset(&mut self, page_info: PageInformation) {
        self.page_info = page_info;
        self.page_pristine = true;
        self.referred_segments.clear();
        self.dictionaries.clear();
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
}

/// Decoded dictionaries and tables, stored as (`segment_number`, value) pairs.
#[derive(Default)]
pub(crate) struct Dictionaries {
    pattern_dictionaries: Vec<(u32, PatternDictionary)>,
    symbol_dictionaries: Vec<(u32, SymbolDictionary)>,
    huffman_tables: Vec<(u32, HuffmanTable)>,
}

impl Dictionaries {
    fn clear(&mut self) {
        self.pattern_dictionaries.clear();
        self.symbol_dictionaries.clear();
        self.huffman_tables.clear();
    }

    fn store_pattern_dictionary(&mut self, segment_number: u32, dictionary: PatternDictionary) {
        self.pattern_dictionaries.push((segment_number, dictionary));
    }

    fn store_symbol_dictionary(&mut self, segment_number: u32, dictionary: SymbolDictionary) {
        self.symbol_dictionaries.push((segment_number, dictionary));
    }

    fn store_huffman_table(&mut self, segment_number: u32, table: HuffmanTable) {
        self.huffman_tables.push((segment_number, table));
    }

    fn get_pattern_dictionary<'a>(
        &'a self,
        segment_number: u32,
        globals: Option<&'a Globals>,
    ) -> Option<&'a PatternDictionary> {
        find(&self.pattern_dictionaries, segment_number)
            .or_else(|| find(&globals?.dictionaries.pattern_dictionaries, segment_number))
    }

    fn get_symbol_dictionary<'a>(
        &'a self,
        segment_number: u32,
        globals: Option<&'a Globals>,
    ) -> Option<&'a SymbolDictionary> {
        find(&self.symbol_dictionaries, segment_number)
            .or_else(|| find(&globals?.dictionaries.symbol_dictionaries, segment_number))
    }

    fn get_huffman_table<'a>(
        &'a self,
        segment_number: u32,
        globals: Option<&'a Globals>,
    ) -> Option<&'a HuffmanTable> {
        find(&self.huffman_tables, segment_number)
            .or_else(|| find(&globals?.dictionaries.huffman_tables, segment_number))
    }

    fn get_symbols_and_tables<'a>(
        &'a self,
        referred_segments: &[u32],
        globals: Option<&'a Globals>,
        strict: bool,
    ) -> Result<(Vec<&'a Bitmap>, Vec<HuffmanTable>)> {
        let mut symbols = Vec::new();
        let mut tables = Vec::new();

        for &num in referred_segments {
            if let Some(dictionary) = self.get_symbol_dictionary(num, globals) {
                if symbols.len() + dictionary.exported_symbols.len() > MAX_SYMBOLS as usize {
                    bail!(OverflowError::SymbolCount);
                }

                symbols.extend(dictionary.exported_symbols.iter());
            } else if let Some(table) = self.get_huffman_table(num, globals) {
                tables.push(table.clone());
            } else if strict {
                bail!(SegmentError::InvalidReference);
            }
        }

        Ok((symbols, tables))
    }
}

fn find<T>(entries: &[(u32, T)], segment_number: u32) -> Option<&T> {
    entries
        .binary_search_by_key(&segment_number, |(num, _)| *num)
        .ok()
        .map(|idx| &entries[idx].1)
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
