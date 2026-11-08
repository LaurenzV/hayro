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
use crate::memory::MemoryBudget;

pub(crate) struct DecodeState<'a> {
    pub(crate) contexts: Vec<ArithmeticDecoderContext>,
    pub(crate) budget: MemoryBudget,
    pub(crate) cancel: Cancel<'a>,
}

impl<'a> DecodeState<'a> {
    fn new(memory_limit: Option<usize>, cancel: Cancel<'a>) -> Self {
        Self {
            contexts: Vec::new(),
            budget: MemoryBudget::new(memory_limit),
            cancel,
        }
    }
}

#[cfg(feature = "image")]
pub mod integration;

mod arithmetic_decoder;
mod bitmap;
mod cancel;
mod decode;
mod error;
mod file;
mod gray_scale;
mod huffman_table;
mod integer_decoder;
mod lazy;
mod memory;
mod page_info;
mod reader;
mod segment;
mod simd;
mod symbol_id_decoder;
mod sync;

use error::bail;
pub use error::{
    Cancelled, DecodeError, FormatError, HuffmanError, OverflowError, ParseError, RegionError,
    Result, SegmentError, SymbolError, TemplateError,
};

use crate::file::parse_segments_sequential;
use bitmap::Bitmap;
use cancel::{Cancel, FILL_CHUNK, copy_work, row_mask};
pub use cancel::{CancelCheck, CancelHandle};
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
use page_info::parse_page_information;
use reader::Reader;
use segment::{Segment, SegmentType};
use sync::{Arc, OnceLock};

/// Settings for decoding JBIG2 images.
#[derive(Debug, Clone)]
pub struct DecodeSettings {
    /// Whether to reject data that violates the specification, instead of
    /// decoding it as well as possible.
    ///
    /// Defaults to `false`.
    pub strict: bool,
    /// The maximum number of bytes that decoding a page may hold at once, or
    /// `None` for no limit.
    ///
    /// Defaults to 1 GiB.
    pub memory_limit: Option<usize>,
    /// A check for cancelling decoding.
    pub cancel: Option<CancelHandle>,
}

impl Default for DecodeSettings {
    fn default() -> Self {
        Self {
            strict: false,
            memory_limit: Some(1 << 30),
            cancel: None,
        }
    }
}

/// A JBIG2 file in the sequential or random-access organization (Annex D.1, D.2).
pub struct File<'a> {
    pages: Vec<FilePage<'a>>,
    globals: Option<LazyGlobals<'a>>,
    settings: DecodeSettings,
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
    fn get(&self, settings: &DecodeSettings) -> Result<&Globals> {
        if let Some(decoded) = self.decoded.get() {
            return decoded.as_ref().map_err(|e| *e);
        }

        let decoded = Globals::from_segments(&self.segments, settings);
        if matches!(decoded, Err(DecodeError::Cancelled)) {
            return Err(DecodeError::Cancelled);
        }

        self.decoded
            .get_or_init(|| decoded)
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
            settings: settings.clone(),
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
            settings: self.settings.clone(),
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

        Self::from_segments(&segments, settings)
    }

    fn from_segments(segments: &[Segment<'_>], settings: &DecodeSettings) -> Result<Self> {
        let strict = settings.strict;
        let mut dictionaries = Dictionaries::default();
        let standard_tables = StandardHuffmanTables::default();
        let cancel = Cancel(settings.cancel.as_ref());
        let mut state = DecodeState::new(None, cancel);

        for seg in segments {
            cancel.check()?;
            decode_dictionary_segment(
                seg,
                &mut dictionaries,
                None,
                &standard_tables,
                &mut state,
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
    settings: DecodeSettings,
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
            settings: settings.clone(),
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

    /// Decode the page.
    pub fn decode(&self) -> Result<PageBitmap> {
        let segments = match &self.segments {
            PageSegments::Owned(segments) => segments,
            PageSegments::Borrowed(segments) => *segments,
        };
        let globals = match self.globals {
            Some(GlobalsRef::Decoded(globals)) => Some(globals),
            Some(GlobalsRef::Lazy(globals)) => Some(globals.get(&self.settings)?),
            None => None,
        };

        let bitmap = decode_segments(segments, globals, self.size, &self.settings)?;

        Ok(PageBitmap {
            bitmap,
            cancel: self.settings.cancel.clone(),
        })
    }
}

/// The decoded pixels of a page, as rows of `u32` words with the first pixel in the most
/// significant bit and black pixels set to 1.
pub struct PageBitmap {
    bitmap: Bitmap,
    cancel: Option<CancelHandle>,
}

impl PageBitmap {
    /// The width of the bitmap in pixels.
    pub fn width(&self) -> u32 {
        self.bitmap.width
    }

    /// The height of the bitmap in pixels.
    pub fn height(&self) -> u32 {
        self.bitmap.height
    }

    /// The rows of the bitmap. Bits past the width of the bitmap are unspecified.
    pub fn rows(&self) -> core::slice::ChunksExact<'_, u32> {
        self.bitmap.data.chunks_exact(self.bitmap.stride as usize)
    }

    /// Return the pixels as 8-bit samples, with black as 0 and white as 255.
    pub fn data_u8(&self) -> core::result::Result<Vec<u8>, Cancelled> {
        let cancel = Cancel(self.cancel.as_ref());
        let len = self.width() as usize * self.height() as usize;
        let mut buf = Vec::with_capacity(len);

        while buf.len() < len {
            cancel.check()?;
            buf.resize(len.min(buf.len() + FILL_CHUNK), 0xFF);
        }

        self.store_black_u8(&mut buf, cancel)?;

        Ok(buf)
    }

    /// Store the pixels as 8-bit samples into `buf`, which must have `width × height` bytes.
    pub fn store_u8_into(&self, buf: &mut [u8]) -> core::result::Result<(), Cancelled> {
        assert_eq!(
            buf.len(),
            self.width() as usize * self.height() as usize,
            "buffer has the wrong size"
        );

        let cancel = Cancel(self.cancel.as_ref());

        for chunk in buf.chunks_mut(FILL_CHUNK) {
            cancel.check()?;
            chunk.fill(0xFF);
        }

        self.store_black_u8(buf, cancel)
    }

    fn store_black_u8(
        &self,
        buf: &mut [u8],
        cancel: Cancel<'_>,
    ) -> core::result::Result<(), Cancelled> {
        let mask = row_mask(copy_work(self.width())) as usize;

        for (y, (row, out)) in self
            .rows()
            .zip(buf.chunks_exact_mut(self.width() as usize))
            .enumerate()
        {
            if y & mask == mask {
                cancel.check()?;
            }

            for (word, out) in row.iter().zip(out.chunks_mut(32)) {
                if *word == 0 {
                    continue;
                }

                for (byte, out) in word.to_be_bytes().into_iter().zip(out.chunks_mut(8)) {
                    if byte != 0 {
                        out.copy_from_slice(&U8_PIXELS[byte as usize][..out.len()]);
                    }
                }
            }
        }

        Ok(())
    }
}

static U8_PIXELS: [[u8; 8]; 256] = {
    let mut table = [[0xFF; 8]; 256];
    let mut byte = 0;

    while byte < 256 {
        let mut bit = 0;

        while bit < 8 {
            if byte & (0x80 >> bit) != 0 {
                table[byte][bit] = 0x00;
            }

            bit += 1;
        }

        byte += 1;
    }

    table
};

#[derive(Debug, Clone, Copy, Default)]
struct PageSize {
    width: u32,
    height: u32,
    default_pixel: bool,
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

        bitmap::check_size(page_info.width, height)?;

        Ok(Self {
            width: page_info.width,
            height,
            default_pixel: page_info.flags.default_pixel != 0,
        })
    }
}

fn decode_segments(
    segments: &[Segment<'_>],
    globals: Option<&Globals>,
    size: PageSize,
    settings: &DecodeSettings,
) -> Result<Bitmap> {
    let strict = settings.strict;
    let cancel = Cancel(settings.cancel.as_ref());
    let mut state = DecodeState::new(settings.memory_limit, cancel);
    // "Bit 2: Page default pixel value. This bit contains the initial value
    // for every pixel in the page, before any region segments are decoded
    // or drawn." (7.4.8.5)
    let mut page_bitmap = Bitmap::new_with(
        size.width,
        size.height,
        0,
        0,
        size.default_pixel,
        &mut state.budget,
    )?;
    state.budget.retain(page_bitmap.memory_size());
    let mut page_state = PageState::new(size.default_pixel);
    let standard_tables = StandardHuffmanTables::default();

    for seg in segments {
        cancel.check()?;
        let result = decode_segment(
            seg,
            &mut page_bitmap,
            &mut page_state,
            globals,
            &standard_tables,
            &mut state,
            strict,
        );
        state.budget.end_segment();

        match result {
            Ok(ControlFlow::Continue(())) => {}
            Ok(ControlFlow::Break(())) => break,
            // No other segment can refer to an immediate region, so the rest of
            // the page doesn't depend on it.
            Err(e)
                if !strict
                    && e != DecodeError::MemoryLimitExceeded
                    && e != DecodeError::Cancelled
                    && seg.header.segment_type.is_immediate_region() =>
            {
                // It might have been decoded directly into the page.
                page_state.page_pristine = false;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(page_bitmap)
}

fn decode_segment(
    seg: &Segment<'_>,
    page_bitmap: &mut Bitmap,
    page_state: &mut PageState,
    globals: Option<&Globals>,
    standard_tables: &StandardHuffmanTables,
    state: &mut DecodeState<'_>,
    strict: bool,
) -> Result<ControlFlow<()>> {
    let mut reader = Reader::new(seg.data);

    match seg.header.segment_type {
        SegmentType::PageInformation => {}
        SegmentType::ImmediateGenericRegion | SegmentType::ImmediateLosslessGenericRegion => {
            let had_unknown_length = seg.header.data_length.is_none();
            let header = generic::parse(&mut reader, had_unknown_length, strict)?;

            if page_state.can_decode_directly(page_bitmap, &header.region_info, false) {
                generic::decode_into(&header, page_bitmap, state)?;
            } else {
                let region = generic::decode(&header, state)?;
                page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                    state.cancel,
                )?;
            }
            page_state.page_pristine = false;
        }
        SegmentType::IntermediateGenericRegion => {
            // Intermediate segments cannot have unknown length.
            let header = generic::parse(&mut reader, false, strict)?;
            let region = generic::decode(&header, state)?;
            page_state.store_region(seg.header.segment_number, region.bitmap, &mut state.budget);
        }
        SegmentType::PatternDictionary
        | SegmentType::SymbolDictionary
        | SegmentType::Tables
        | SegmentType::Extension => decode_dictionary_segment(
            seg,
            &mut page_state.dictionaries,
            globals,
            standard_tables,
            state,
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
                    standard_tables,
                    page_bitmap,
                    state,
                )?;
            } else {
                let region =
                    text::decode(&header, &symbols, &referred_tables, standard_tables, state)?;
                page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                    state.cancel,
                )?;
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
            let region = text::decode(&header, &symbols, &referred_tables, standard_tables, state)?;
            page_state.store_region(seg.header.segment_number, region.bitmap, &mut state.budget);
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
                halftone::decode_into(&header, pattern_dict, page_bitmap, state)?;
            } else {
                let region = halftone::decode(&header, pattern_dict, state)?;
                page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                    state.cancel,
                )?;
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
            let region = halftone::decode(&header, pattern_dict, state)?;
            page_state.store_region(seg.header.segment_number, region.bitmap, &mut state.budget);
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
                    page_area = page_bitmap_area(
                        page_bitmap,
                        &header.region_info,
                        &mut state.budget,
                        state.cancel,
                    )?;
                    &page_area
                }
            };
            let region = generic_refinement::decode(&header, reference, state)?;

            page_state.store_region(seg.header.segment_number, region.bitmap, &mut state.budget);
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
                generic_refinement::decode_into(&header, referred_segment, page_bitmap, state)?;
            } else {
                let page_area;
                let reference = match referred_segment {
                    Some(referred_segment) => referred_segment,
                    None => {
                        page_area = page_bitmap_area(
                            page_bitmap,
                            &header.region_info,
                            &mut state.budget,
                            state.cancel,
                        )?;
                        &page_area
                    }
                };
                let region = generic_refinement::decode(&header, reference, state)?;
                page_bitmap.combine(
                    &region.bitmap,
                    region.bitmap.x_location as i32,
                    region.bitmap.y_location as i32,
                    region.combination_operator,
                    state.cancel,
                )?;
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
    state: &mut DecodeState<'_>,
    strict: bool,
) -> Result<()> {
    let mut reader = Reader::new(seg.data);

    match seg.header.segment_type {
        SegmentType::PatternDictionary => {
            let header = pattern::parse(&mut reader, strict)?;
            let dictionary = pattern::decode(&header, state)?;
            state
                .budget
                .retain(dictionary.patterns.iter().map(Bitmap::memory_size).sum());
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
                &mut state.budget,
                state.cancel,
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
pub(crate) struct PageState {
    /// The page's default pixel value.
    pub(crate) default_pixel: bool,
    /// Whether the page bitmap is still in its initial state (not yet painted to).
    pub(crate) page_pristine: bool,
    /// Decoded intermediate regions, stored as (`segment_number`, region) pairs.
    pub(crate) referred_segments: Vec<(u32, Bitmap)>,
    /// The dictionaries and tables of the page.
    pub(crate) dictionaries: Dictionaries,
}

impl PageState {
    fn new(default_pixel: bool) -> Self {
        Self {
            default_pixel,
            page_pristine: true,
            referred_segments: Vec::new(),
            dictionaries: Dictionaries::default(),
        }
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

        let page_default_is_zero = !self.default_pixel;

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
    fn store_region(&mut self, segment_number: u32, region: Bitmap, budget: &mut MemoryBudget) {
        budget.retain(region.memory_size());
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
    ) -> Result<(Vec<&'a Arc<Bitmap>>, Vec<HuffmanTable>)> {
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

fn page_bitmap_area(
    page: &Bitmap,
    region_info: &decode::RegionSegmentInfo,
    budget: &mut MemoryBudget,
    cancel: Cancel<'_>,
) -> Result<Bitmap> {
    let mut area = Bitmap::new_with(
        region_info.width,
        region_info.height,
        region_info.x_location,
        region_info.y_location,
        false,
        budget,
    )?;
    area.combine(
        page,
        -i32::try_from(region_info.x_location).unwrap_or(i32::MAX),
        -i32::try_from(region_info.y_location).unwrap_or(i32::MAX),
        CombinationOperator::Replace,
        cancel,
    )?;

    Ok(area)
}
