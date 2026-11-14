use alloc::vec::Vec;
use core::iter;

use super::{
    AdaptiveTemplatePixel, CombinationOperator, RefinementTemplate, RegionSegmentInfo,
    parse_refinement_at_pixels, parse_region_segment_info,
};
use super::{RegionBitmap, generic_refinement};
use crate::DecodeState;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::Bitmap;
use crate::cancel::{Cancel, WORD_WORK, WorkCounter};
use crate::error::{HuffmanError, OverflowError, ParseError, Result, SymbolError, bail};
use crate::huffman_table::{HuffmanTable, StandardHuffmanTables, TableLine};
use crate::integer_decoder::IntegerDecoder;
use crate::memory::MemoryBudget;
use crate::reader::Reader;
use crate::symbol_id_decoder::SymbolIdDecoder;
use crate::sync::Arc;

pub(crate) fn decode(
    header: &TextRegionHeader<'_>,
    symbols: &[&Arc<Bitmap>],
    referred_tables: &[HuffmanTable],
    standard_tables: &StandardHuffmanTables,
    scratch: &mut DecodeState<'_>,
) -> Result<RegionBitmap> {
    let mut bitmap = Bitmap::new_with(
        header.region_info.width,
        header.region_info.height,
        header.region_info.x_location,
        header.region_info.y_location,
        header.flags.default_pixel,
        &mut scratch.budget,
    )?;

    decode_into(
        header,
        symbols,
        referred_tables,
        standard_tables,
        &mut bitmap,
        scratch,
    )?;

    Ok(RegionBitmap {
        bitmap,
        combination_operator: header.region_info.combination_operator,
    })
}

pub(crate) fn decode_into(
    header: &TextRegionHeader<'_>,
    symbols: &[&Arc<Bitmap>],
    referred_tables: &[HuffmanTable],
    standard_tables: &StandardHuffmanTables,
    bitmap: &mut Bitmap,
    scratch: &mut DecodeState<'_>,
) -> Result<()> {
    let num_gr_contexts = 1 << header.flags.refinement_template.context_bits();
    scratch.contexts.clear();
    scratch
        .contexts
        .resize(num_gr_contexts, ArithmeticDecoderContext::default());

    if header.flags.use_huffman {
        let mut reader = Reader::new(header.data);
        let ctx = DecodeContext::new_huffman(
            &mut reader,
            header,
            referred_tables,
            standard_tables,
            &mut scratch.contexts,
        )?;
        decode_with(
            ctx,
            SymbolBitmaps::new(symbols, &[]),
            header,
            bitmap,
            &mut scratch.budget,
            scratch.cancel,
        )?;
    } else {
        let mut decoder = ArithmeticDecoder::new(header.data);

        let num_symbols = symbols.len() as u32;
        let symbol_code_length = 32 - num_symbols.saturating_sub(1).leading_zeros();
        let mut contexts = TextRegionContexts::new(symbol_code_length);

        let ctx = DecodeContext::new_arithmetic(&mut decoder, &mut contexts, &mut scratch.contexts);
        decode_with(
            ctx,
            SymbolBitmaps::new(symbols, &[]),
            header,
            bitmap,
            &mut scratch.budget,
            scratch.cancel,
        )?;
    }

    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct SymbolBitmaps<'a> {
    first: &'a [&'a Arc<Bitmap>],
    second: &'a [Arc<Bitmap>],
}

impl<'a> SymbolBitmaps<'a> {
    pub(crate) fn new(first: &'a [&'a Arc<Bitmap>], second: &'a [Arc<Bitmap>]) -> Self {
        Self { first, second }
    }

    fn get(self, index: usize) -> Option<&'a Bitmap> {
        let bitmap = match self.first.get(index) {
            Some(bitmap) => bitmap,
            None => self.second.get(index - self.first.len())?,
        };

        Some(bitmap)
    }
}

pub(crate) fn decode_with(
    mut ctx: DecodeContext<'_, '_>,
    symbols: SymbolBitmaps<'_>,
    header: &TextRegionHeader<'_>,
    region: &mut Bitmap,
    budget: &mut MemoryBudget,
    cancel: Cancel<'_>,
) -> Result<()> {
    let strip_size = header.strip_size();

    let mut strip_t = ctx
        .read_strip_delta_t(strip_size)?
        .checked_neg()
        .ok_or(OverflowError::PlacementCoordinate)?;
    let mut first_s: i32 = 0;
    let mut instance_count = 0;
    let mut work = WorkCounter::new();

    while instance_count < header.num_instances {
        let delta_t = ctx.read_strip_delta_t(strip_size)?;
        strip_t = strip_t
            .checked_add(delta_t)
            .ok_or(OverflowError::PlacementCoordinate)?;

        let mut first_symbol_in_strip = true;
        let mut current_s = 0;

        loop {
            if ctx.is_exhausted() {
                bail!(ParseError::UnexpectedEof);
            }

            work.add(128, cancel)?;

            if first_symbol_in_strip {
                let delta_first_s = ctx.read_first_s()?;
                first_s = first_s
                    .checked_add(delta_first_s)
                    .ok_or(OverflowError::PlacementCoordinate)?;
                current_s = first_s;
                first_symbol_in_strip = false;
            } else if instance_count == header.num_instances {
                let end_of_strip = ctx.read_delta_s();

                if header.strict && end_of_strip?.is_some() {
                    bail!(SymbolError::TooManyInstances);
                }

                break;
            } else {
                let Some(delta_s) = ctx.read_delta_s()? else {
                    break;
                };

                current_s = current_s
                    .checked_add(delta_s)
                    .and_then(|v| v.checked_add(header.flags.delta_s_offset as i32))
                    .ok_or(OverflowError::PlacementCoordinate)?;
            }

            let current_t = ctx.read_symbol_t(strip_size, header.flags.log_strip_size)?;
            let symbol_t = strip_t
                .checked_add(current_t)
                .ok_or(OverflowError::PlacementCoordinate)?;

            let symbol_id = ctx.read_symbol_id()?;

            let symbol_bitmap = decode_symbol_instance_bitmap(
                &mut ctx, symbols, header, symbol_id, budget, cancel,
            )?;

            let symbol_bitmap_ref: &Bitmap = match &symbol_bitmap {
                SymbolBitmap::Reference(idx) => match symbols.get(*idx) {
                    Some(bitmap) => bitmap,
                    None if header.strict => bail!(SymbolError::OutOfRange),
                    None => {
                        instance_count += 1;
                        continue;
                    }
                },
                SymbolBitmap::Owned(bitmap) => bitmap,
            };
            let symbol_width = symbol_bitmap_ref.width as i32;
            let symbol_height = symbol_bitmap_ref.height as i32;

            if !header.flags.transposed
                && (header.flags.reference_corner == ReferenceCorner::TopRight
                    || header.flags.reference_corner == ReferenceCorner::BottomRight)
            {
                current_s = current_s
                    .checked_add(symbol_width - 1)
                    .ok_or(OverflowError::PlacementCoordinate)?;
            } else if header.flags.transposed
                && (header.flags.reference_corner == ReferenceCorner::BottomLeft
                    || header.flags.reference_corner == ReferenceCorner::BottomRight)
            {
                current_s = current_s
                    .checked_add(symbol_height - 1)
                    .ok_or(OverflowError::PlacementCoordinate)?;
            }

            let symbol_s = current_s;

            let start = |end: i32, size: i32| {
                end.checked_sub(size - 1)
                    .ok_or(OverflowError::PlacementCoordinate)
            };
            let (x, y) = if !header.flags.transposed {
                match header.flags.reference_corner {
                    ReferenceCorner::TopLeft => (symbol_s, symbol_t),
                    ReferenceCorner::TopRight => (start(symbol_s, symbol_width)?, symbol_t),
                    ReferenceCorner::BottomLeft => (symbol_s, start(symbol_t, symbol_height)?),
                    ReferenceCorner::BottomRight => (
                        start(symbol_s, symbol_width)?,
                        start(symbol_t, symbol_height)?,
                    ),
                }
            } else {
                match header.flags.reference_corner {
                    ReferenceCorner::TopLeft => (symbol_t, symbol_s),
                    ReferenceCorner::TopRight => (start(symbol_t, symbol_width)?, symbol_s),
                    ReferenceCorner::BottomLeft => (symbol_t, start(symbol_s, symbol_height)?),
                    ReferenceCorner::BottomRight => (
                        start(symbol_t, symbol_width)?,
                        start(symbol_s, symbol_height)?,
                    ),
                }
            };

            region.combine(
                symbol_bitmap_ref,
                x,
                y,
                header.flags.combination_operator,
                cancel,
            )?;
            work.add(
                u64::from(WORD_WORK) * symbol_bitmap_ref.data.len() as u64,
                cancel,
            )?;

            if let SymbolBitmap::Owned(bitmap) = &symbol_bitmap {
                budget.release(bitmap.memory_size());
            }

            if !header.flags.transposed
                && (header.flags.reference_corner == ReferenceCorner::TopLeft
                    || header.flags.reference_corner == ReferenceCorner::BottomLeft)
            {
                current_s = current_s
                    .checked_add(symbol_width - 1)
                    .ok_or(OverflowError::PlacementCoordinate)?;
            } else if header.flags.transposed
                && (header.flags.reference_corner == ReferenceCorner::TopLeft
                    || header.flags.reference_corner == ReferenceCorner::TopRight)
            {
                current_s = current_s
                    .checked_add(symbol_height - 1)
                    .ok_or(OverflowError::PlacementCoordinate)?;
            }

            instance_count += 1;
        }
    }

    Ok(())
}

pub(crate) struct TextRegionContexts {
    pub(crate) iadt: IntegerDecoder,
    pub(crate) iafs: IntegerDecoder,
    pub(crate) iads: IntegerDecoder,
    pub(crate) iait: IntegerDecoder,
    pub(crate) iaid: SymbolIdDecoder,
    pub(crate) iari: IntegerDecoder,
    pub(crate) iardw: IntegerDecoder,
    pub(crate) iardh: IntegerDecoder,
    pub(crate) iardx: IntegerDecoder,
    pub(crate) iardy: IntegerDecoder,
}

impl TextRegionContexts {
    pub(crate) fn new(symbol_code_length: u32) -> Self {
        Self {
            iadt: IntegerDecoder::new(),
            iafs: IntegerDecoder::new(),
            iads: IntegerDecoder::new(),
            iait: IntegerDecoder::new(),
            iaid: SymbolIdDecoder::new(symbol_code_length),
            iari: IntegerDecoder::new(),
            iardw: IntegerDecoder::new(),
            iardh: IntegerDecoder::new(),
            iardx: IntegerDecoder::new(),
            iardy: IntegerDecoder::new(),
        }
    }
}

pub(crate) enum DecodeContext<'a, 'b> {
    Huffman {
        reader: &'a mut Reader<'b>,
        tables: TextRegionHuffmanTables<'a>,
        symbol_codes: &'a HuffmanTable,
        gr_contexts: &'a mut [ArithmeticDecoderContext],
    },
    Arithmetic {
        decoder: &'a mut ArithmeticDecoder<'b>,
        contexts: &'a mut TextRegionContexts,
        gr_contexts: &'a mut [ArithmeticDecoderContext],
    },
}

impl<'a, 'b> DecodeContext<'a, 'b> {
    pub(crate) fn new_huffman(
        reader: &'a mut Reader<'b>,
        header: &'a TextRegionHeader<'_>,
        referred_tables: &'a [HuffmanTable],
        standard_tables: &'a StandardHuffmanTables,
        gr_contexts: &'a mut [ArithmeticDecoderContext],
    ) -> Result<Self> {
        let huffman_flags = header
            .huffman_flags
            .as_ref()
            .ok_or(HuffmanError::InvalidSelection)?;
        let tables = select_huffman_tables(huffman_flags, referred_tables, standard_tables)?;
        let symbol_codes = header
            .symbol_id_table
            .as_ref()
            .ok_or(HuffmanError::MissingTables)?;

        Ok(DecodeContext::Huffman {
            reader,
            tables,
            symbol_codes,
            gr_contexts,
        })
    }

    pub(crate) fn new_arithmetic(
        decoder: &'a mut ArithmeticDecoder<'b>,
        contexts: &'a mut TextRegionContexts,
        gr_contexts: &'a mut [ArithmeticDecoderContext],
    ) -> Self {
        DecodeContext::Arithmetic {
            decoder,
            contexts,
            gr_contexts,
        }
    }

    fn is_exhausted(&self) -> bool {
        match self {
            DecodeContext::Huffman { .. } => false,
            DecodeContext::Arithmetic { decoder, .. } => decoder.is_exhausted(),
        }
    }

    fn read_strip_delta_t(&mut self, strip_size: u32) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                let value = tables.delta_t.decode_no_oob(reader)?;
                value
                    .checked_mul(strip_size as i32)
                    .ok_or(SymbolError::InvalidStripDelta.into())
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => {
                let value = contexts
                    .iadt
                    .decode(decoder)
                    .ok_or(SymbolError::UnexpectedOob)?;
                value
                    .checked_mul(strip_size as i32)
                    .ok_or(SymbolError::InvalidStripDelta.into())
            }
        }
    }

    fn read_first_s(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => tables.first_s.decode_no_oob(reader),
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iafs
                .decode(decoder)
                .ok_or(SymbolError::UnexpectedOob.into()),
        }
    }

    fn read_delta_s(&mut self) -> Result<Option<i32>> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => tables.delta_s.decode(reader),
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => Ok(contexts.iads.decode(decoder)),
        }
    }

    fn read_symbol_t(&mut self, strip_size: u32, log_strip_size: u8) -> Result<i32> {
        if strip_size == 1 {
            return Ok(0);
        }

        match self {
            DecodeContext::Huffman { reader, .. } => reader
                .read_bits(log_strip_size)
                .ok_or(HuffmanError::InvalidCode.into())
                .map(|v| v as i32),
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iait
                .decode(decoder)
                .ok_or(SymbolError::UnexpectedOob.into()),
        }
    }

    fn read_symbol_id(&mut self) -> Result<usize> {
        match self {
            DecodeContext::Huffman {
                reader,
                symbol_codes,
                ..
            } => symbol_codes.decode_no_oob(reader).map(|v| v as usize),
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => Ok(contexts.iaid.decode(decoder) as usize),
        }
    }

    fn read_refinement_flag(&mut self) -> Result<u8> {
        match self {
            DecodeContext::Huffman { reader, .. } => {
                reader.read_bit().ok_or(ParseError::UnexpectedEof.into())
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => Ok(contexts
                .iari
                .decode(decoder)
                .ok_or(SymbolError::UnexpectedOob)? as u8),
        }
    }

    fn read_refinement_delta_width(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                tables.refinement_width.decode_no_oob(reader)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iardw
                .decode(decoder)
                .ok_or(SymbolError::UnexpectedOob.into()),
        }
    }

    fn read_refinement_delta_height(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                tables.refinement_height.decode_no_oob(reader)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iardh
                .decode(decoder)
                .ok_or(SymbolError::UnexpectedOob.into()),
        }
    }

    fn read_refinement_x_offset(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                tables.refinement_x.decode_no_oob(reader)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iardx
                .decode(decoder)
                .ok_or(SymbolError::UnexpectedOob.into()),
        }
    }

    fn read_refinement_y_offset(&mut self) -> Result<i32> {
        match self {
            DecodeContext::Huffman { reader, tables, .. } => {
                tables.refinement_y.decode_no_oob(reader)
            }
            DecodeContext::Arithmetic {
                decoder, contexts, ..
            } => contexts
                .iardy
                .decode(decoder)
                .ok_or(SymbolError::UnexpectedOob.into()),
        }
    }

    fn decode_refinement_bitmap(
        &mut self,
        refined: &mut Bitmap,
        reference_bitmap: &Bitmap,
        reference_x_offset: i32,
        reference_y_offset: i32,
        refinement_template: RefinementTemplate,
        refinement_at_pixels: &[AdaptiveTemplatePixel],
        cancel: Cancel<'_>,
    ) -> Result<()> {
        match self {
            DecodeContext::Huffman {
                reader,
                tables,
                gr_contexts,
                ..
            } => {
                let refinement_data_size = tables.refinement_size.decode_no_oob(reader)? as u32;
                reader.align();

                let refinement_data = reader
                    .read_bytes(refinement_data_size as usize)
                    .ok_or(ParseError::UnexpectedEof)?;

                let mut decoder = ArithmeticDecoder::new(refinement_data);

                generic_refinement::decode_bitmap(
                    &mut decoder,
                    gr_contexts,
                    refined,
                    reference_bitmap,
                    reference_x_offset,
                    reference_y_offset,
                    refinement_template,
                    refinement_at_pixels,
                    false,
                    cancel,
                )
            }
            DecodeContext::Arithmetic {
                decoder,
                gr_contexts,
                ..
            } => generic_refinement::decode_bitmap(
                decoder,
                gr_contexts,
                refined,
                reference_bitmap,
                reference_x_offset,
                reference_y_offset,
                refinement_template,
                refinement_at_pixels,
                false,
                cancel,
            ),
        }
    }
}

enum SymbolBitmap {
    Reference(usize),
    Owned(Bitmap),
}

fn decode_symbol_instance_bitmap(
    ctx: &mut DecodeContext<'_, '_>,
    symbols: SymbolBitmaps<'_>,
    header: &TextRegionHeader<'_>,
    symbol_id: usize,
    budget: &mut MemoryBudget,
    cancel: Cancel<'_>,
) -> Result<SymbolBitmap> {
    if !header.flags.use_refinement || ctx.read_refinement_flag()? == 0 {
        return Ok(SymbolBitmap::Reference(symbol_id));
    }

    let reference_bitmap = symbols.get(symbol_id).ok_or(SymbolError::OutOfRange)?;

    let rdw = ctx.read_refinement_delta_width()?;
    let rdh = ctx.read_refinement_delta_height()?;
    let rdx = ctx.read_refinement_x_offset()?;
    let rdy = ctx.read_refinement_y_offset()?;

    let refined_width = (reference_bitmap.width as i32)
        .checked_add(rdw)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(OverflowError::BitmapDimension)?;
    let refined_height = (reference_bitmap.height as i32)
        .checked_add(rdh)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(OverflowError::BitmapDimension)?;
    let reference_x_offset = rdw
        .div_euclid(2)
        .checked_add(rdx)
        .ok_or(OverflowError::ReferenceOffset)?;
    let reference_y_offset = rdh
        .div_euclid(2)
        .checked_add(rdy)
        .ok_or(OverflowError::ReferenceOffset)?;

    let mut refined_bitmap = Bitmap::new(refined_width, refined_height, budget)?;

    ctx.decode_refinement_bitmap(
        &mut refined_bitmap,
        reference_bitmap,
        reference_x_offset,
        reference_y_offset,
        header.flags.refinement_template,
        &header.refinement_at_pixels,
        cancel,
    )?;

    Ok(SymbolBitmap::Owned(refined_bitmap))
}

fn select_huffman_tables<'a>(
    flags: &TextRegionHuffmanFlags,
    custom_tables: &'a [HuffmanTable],
    standard_tables: &'a StandardHuffmanTables,
) -> Result<TextRegionHuffmanTables<'a>> {
    let mut custom_table_idx = 0;

    let mut get_custom = || -> Result<&'a HuffmanTable> {
        let table = custom_tables
            .get(custom_table_idx)
            .ok_or(HuffmanError::MissingTables)?;
        custom_table_idx += 1;
        Ok(table)
    };

    let first_s = match flags.first_s_table {
        0 => standard_tables.table_f(),
        1 => standard_tables.table_g(),
        3 => get_custom()?,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let delta_s = match flags.delta_s_table {
        0 => standard_tables.table_h(),
        1 => standard_tables.table_i(),
        2 => standard_tables.table_j(),
        3 => get_custom()?,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let delta_t = match flags.delta_t_table {
        0 => standard_tables.table_k(),
        1 => standard_tables.table_l(),
        2 => standard_tables.table_m(),
        3 => get_custom()?,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let refinement_width = match flags.refinement_width_table {
        0 => standard_tables.table_n(),
        1 => standard_tables.table_o(),
        3 => get_custom()?,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let refinement_height = match flags.refinement_height_table {
        0 => standard_tables.table_n(),
        1 => standard_tables.table_o(),
        3 => get_custom()?,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let refinement_x = match flags.refinement_x_table {
        0 => standard_tables.table_n(),
        1 => standard_tables.table_o(),
        3 => get_custom()?,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let refinement_y = match flags.refinement_y_table {
        0 => standard_tables.table_n(),
        1 => standard_tables.table_o(),
        3 => get_custom()?,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let refinement_size = match flags.refinement_size_table {
        0 => standard_tables.table_a(),
        1 => get_custom()?,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    Ok(TextRegionHuffmanTables {
        first_s,
        delta_s,
        delta_t,
        refinement_width,
        refinement_height,
        refinement_x,
        refinement_y,
        refinement_size,
    })
}

fn decode_symbol_id_huffman_table(
    reader: &mut Reader<'_>,
    num_symbols: u32,
) -> Result<HuffmanTable> {
    let mut runcode_lines: Vec<TableLine> = Vec::with_capacity(35);
    for runcode_idx in 0..35 {
        let prefix_length = reader.read_bits(4).ok_or(HuffmanError::InvalidCode)? as u8;
        runcode_lines.push(TableLine::new(runcode_idx, prefix_length, 0));
    }

    let runcode_table = HuffmanTable::build(&runcode_lines)?;
    let mut symbol_code_lengths = Vec::with_capacity(num_symbols as usize);

    while symbol_code_lengths.len() < num_symbols as usize {
        let runcode = runcode_table.decode_no_oob(reader)? as u32;

        match runcode {
            0..=31 => {
                symbol_code_lengths.push(runcode as u8);
            }
            32 => {
                let extra_bits = reader.read_bits(2).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 3;
                let previous_length = *symbol_code_lengths
                    .last()
                    .ok_or(HuffmanError::InvalidCode)?;
                symbol_code_lengths.extend(iter::repeat_n(previous_length, repeat));
            }
            33 => {
                let extra_bits = reader.read_bits(3).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 3;
                symbol_code_lengths.extend(iter::repeat_n(0, repeat));
            }
            34 => {
                let extra_bits = reader.read_bits(7).ok_or(HuffmanError::InvalidCode)? as usize;
                let repeat = extra_bits + 11;
                symbol_code_lengths.extend(iter::repeat_n(0, repeat));
            }
            _ => bail!(HuffmanError::InvalidCode),
        }
    }

    if symbol_code_lengths.len() != num_symbols as usize {
        bail!(HuffmanError::InvalidCode);
    }

    reader.align();

    let symbol_lines: Vec<TableLine> = symbol_code_lengths
        .iter()
        .enumerate()
        .map(|(symbol_idx, &prefix_length)| TableLine::new(symbol_idx as i32, prefix_length, 0))
        .collect();
    HuffmanTable::build(&symbol_lines)
}

pub(crate) struct TextRegionHuffmanTables<'a> {
    first_s: &'a HuffmanTable,
    delta_s: &'a HuffmanTable,
    delta_t: &'a HuffmanTable,
    refinement_width: &'a HuffmanTable,
    refinement_height: &'a HuffmanTable,
    refinement_x: &'a HuffmanTable,
    refinement_y: &'a HuffmanTable,
    refinement_size: &'a HuffmanTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceCorner {
    BottomLeft,
    TopLeft,
    BottomRight,
    TopRight,
}

impl ReferenceCorner {
    fn from_byte(value: u8) -> Self {
        match value {
            0 => Self::BottomLeft,
            1 => Self::TopLeft,
            2 => Self::BottomRight,
            3 => Self::TopRight,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TextRegionFlags {
    pub(crate) use_huffman: bool,
    pub(crate) use_refinement: bool,
    pub(crate) log_strip_size: u8,
    pub(crate) reference_corner: ReferenceCorner,
    pub(crate) transposed: bool,
    pub(crate) combination_operator: CombinationOperator,
    pub(crate) default_pixel: bool,
    pub(crate) delta_s_offset: i8,
    pub(crate) refinement_template: RefinementTemplate,
}

#[derive(Debug, Clone)]
pub(crate) struct TextRegionHuffmanFlags {
    pub(crate) first_s_table: u8,
    pub(crate) delta_s_table: u8,
    pub(crate) delta_t_table: u8,
    pub(crate) refinement_width_table: u8,
    pub(crate) refinement_height_table: u8,
    pub(crate) refinement_x_table: u8,
    pub(crate) refinement_y_table: u8,
    pub(crate) refinement_size_table: u8,
}

#[derive(Debug, Clone)]
pub(crate) struct TextRegionHeader<'a> {
    pub(crate) region_info: RegionSegmentInfo,
    pub(crate) flags: TextRegionFlags,
    pub(crate) huffman_flags: Option<TextRegionHuffmanFlags>,
    pub(crate) refinement_at_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) num_instances: u32,
    pub(crate) symbol_id_table: Option<HuffmanTable>,
    pub(crate) data: &'a [u8],
    pub(crate) strict: bool,
}

impl TextRegionHeader<'_> {
    pub(crate) fn strip_size(&self) -> u32 {
        1_u32 << self.flags.log_strip_size
    }
}

fn parse_text_region_flags(reader: &mut Reader<'_>) -> Result<TextRegionFlags> {
    let flags_word = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let use_huffman = flags_word & 0x0001 != 0;
    let use_refinement = flags_word & 0x0002 != 0;
    let log_strip_size = ((flags_word >> 2) & 0x03) as u8;
    let reference_corner = ReferenceCorner::from_byte(((flags_word >> 4) & 0x03) as u8);
    let transposed = flags_word & 0x0040 != 0;
    let combination_operator = CombinationOperator::from_value(((flags_word >> 7) & 0x03) as u8)?;

    let default_pixel = flags_word & 0x0200 != 0;

    let delta_s_offset_raw = ((flags_word >> 10) & 0x1F) as u8;
    let delta_s_offset = if delta_s_offset_raw & 0x10 != 0 {
        (delta_s_offset_raw | 0xE0) as i8
    } else {
        delta_s_offset_raw as i8
    };

    let refinement_template = RefinementTemplate::from_byte((flags_word >> 15) as u8);

    Ok(TextRegionFlags {
        use_huffman,
        use_refinement,
        log_strip_size,
        reference_corner,
        transposed,
        combination_operator,
        default_pixel,
        delta_s_offset,
        refinement_template,
    })
}

fn parse_text_region_huffman_flags(reader: &mut Reader<'_>) -> Result<TextRegionHuffmanFlags> {
    let flags_word = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let first_s_table = (flags_word & 0x03) as u8;
    let delta_s_table = ((flags_word >> 2) & 0x03) as u8;
    let delta_t_table = ((flags_word >> 4) & 0x03) as u8;
    let refinement_width_table = ((flags_word >> 6) & 0x03) as u8;
    let refinement_height_table = ((flags_word >> 8) & 0x03) as u8;
    let refinement_x_table = ((flags_word >> 10) & 0x03) as u8;
    let refinement_y_table = ((flags_word >> 12) & 0x03) as u8;
    let refinement_size_table = ((flags_word >> 14) & 0x01) as u8;

    Ok(TextRegionHuffmanFlags {
        first_s_table,
        delta_s_table,
        delta_t_table,
        refinement_width_table,
        refinement_height_table,
        refinement_x_table,
        refinement_y_table,
        refinement_size_table,
    })
}

pub(crate) fn parse<'a>(
    reader: &mut Reader<'a>,
    num_symbols: u32,
    strict: bool,
) -> Result<TextRegionHeader<'a>> {
    let region_info = parse_region_segment_info(reader)?;
    let flags = parse_text_region_flags(reader)?;
    let huffman_flags = if flags.use_huffman {
        Some(parse_text_region_huffman_flags(reader)?)
    } else {
        None
    };

    let refinement_at_pixels =
        if flags.use_refinement && flags.refinement_template == RefinementTemplate::Template0 {
            parse_refinement_at_pixels(reader)?
        } else {
            Vec::new()
        };

    let num_instances = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    let symbol_id_table = if flags.use_huffman {
        Some(decode_symbol_id_huffman_table(reader, num_symbols)?)
    } else {
        None
    };

    let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    Ok(TextRegionHeader {
        region_info,
        flags,
        huffman_flags,
        refinement_at_pixels,
        num_instances,
        symbol_id_table,
        data,
        strict,
    })
}
