//! Symbol dictionary segment parsing and decoding (7.4.2, 6.5).

use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::decode::CombinationOperator;
use crate::decode::generic::{decode_bitmap_mmr, gather_context, parse_adaptive_template_pixels};
use crate::decode::generic_refinement::decode_bitmap;
use crate::decode::text::{
    ReferenceCorner, SymbolBitmap, TextRegionContexts, TextRegionParams, decode_text_region_with,
};
use crate::decode::{
    AdaptiveTemplatePixel, RefinementTemplate, Template, parse_refinement_at_pixels,
};
use crate::error::{DecodeError, HuffmanError, ParseError, RegionError, Result, SymbolError, bail};
use crate::huffman_table::{HuffmanTable, StandardHuffmanTables};
use crate::integer_decoder::IntegerDecoder;
use crate::reader::Reader;

/// Decode a symbol dictionary segment (7.4.2, 6.5).
pub(crate) fn decode(
    reader: &mut Reader<'_>,
    input_symbols: &[&DecodedRegion],
    referred_tables: &[HuffmanTable],
    standard_tables: &StandardHuffmanTables,
) -> Result<SymbolDictionary> {
    let header = parse(reader)?;

    let exported_symbols = if header.flags.use_huffman {
        decode_symbols_huffman(
            reader,
            &header,
            input_symbols,
            referred_tables,
            standard_tables,
        )
    } else {
        let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;
        if header.flags.use_refagg {
            decode_symbols_refagg(data, &header, input_symbols)
        } else {
            decode_symbols_direct(data, &header, input_symbols)
        }
    }?;

    Ok(SymbolDictionary { exported_symbols })
}

/// Huffman table selection for symbol dictionary fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HuffmanTableSelection {
    TableB1,
    TableB2,
    TableB3,
    TableB4,
    TableB5,
    UserSupplied,
}

/// Parsed symbol dictionary segment flags (7.4.2.1.1).
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionaryFlags {
    pub(crate) use_huffman: bool,
    pub(crate) use_refagg: bool,
    pub(crate) delta_height_table: HuffmanTableSelection,
    pub(crate) delta_width_table: HuffmanTableSelection,
    pub(crate) bitmap_size_table: HuffmanTableSelection,
    pub(crate) aggregate_instance_table: HuffmanTableSelection,
    pub(crate) _bitmap_context_used: bool,
    pub(crate) _bitmap_context_retained: bool,
    pub(crate) template: Template,
    pub(crate) refinement_template: RefinementTemplate,
}

/// Parsed symbol dictionary segment header (7.4.2.1).
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionaryHeader {
    pub(crate) flags: SymbolDictionaryFlags,
    pub(crate) at_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) refinement_at_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) num_exported_symbols: u32,
    pub(crate) num_new_symbols: u32,
}

/// Parse a symbol dictionary segment header (7.4.2.1).
fn parse(reader: &mut Reader<'_>) -> Result<SymbolDictionaryHeader> {
    let flags_word = reader.read_u16().ok_or(ParseError::UnexpectedEof)?;
    let use_huffman = flags_word & 0x0001 != 0;
    let use_refagg = flags_word & 0x0002 != 0;

    let delta_height_table = match (flags_word >> 2) & 0x03 {
        0 => HuffmanTableSelection::TableB4,
        1 => HuffmanTableSelection::TableB5,
        3 => HuffmanTableSelection::UserSupplied,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let delta_width_table = match (flags_word >> 4) & 0x03 {
        0 => HuffmanTableSelection::TableB2,
        1 => HuffmanTableSelection::TableB3,
        3 => HuffmanTableSelection::UserSupplied,
        _ => bail!(HuffmanError::InvalidSelection),
    };

    let bitmap_size_table = if flags_word & 0x0040 != 0 {
        HuffmanTableSelection::UserSupplied
    } else {
        HuffmanTableSelection::TableB1
    };

    let aggregate_instance_table = if flags_word & 0x0080 != 0 {
        HuffmanTableSelection::UserSupplied
    } else {
        HuffmanTableSelection::TableB1
    };

    let bitmap_context_used = flags_word & 0x0100 != 0;
    let bitmap_context_retained = flags_word & 0x0200 != 0;
    let template = Template::from_byte((flags_word >> 10) as u8);
    let refinement_template = RefinementTemplate::from_byte((flags_word >> 12) as u8);

    let flags = SymbolDictionaryFlags {
        use_huffman,
        use_refagg,
        delta_height_table,
        delta_width_table,
        bitmap_size_table,
        aggregate_instance_table,
        // TODO: Implement those.
        _bitmap_context_used: bitmap_context_used,
        _bitmap_context_retained: bitmap_context_retained,
        template,
        refinement_template,
    };

    let at_pixels = if !use_huffman {
        parse_adaptive_template_pixels(reader, template, false)?
    } else {
        Vec::new()
    };

    let refinement_at_pixels = if use_refagg && refinement_template == RefinementTemplate::Template0
    {
        parse_refinement_at_pixels(reader)?
    } else {
        Vec::new()
    };
    let num_exported_symbols = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let num_new_symbols = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;

    Ok(SymbolDictionaryHeader {
        flags,
        at_pixels,
        refinement_at_pixels,
        num_exported_symbols,
        num_new_symbols,
    })
}

/// A decoded symbol dictionary segment.
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionary {
    pub(crate) exported_symbols: Vec<DecodedRegion>,
}

/// Decode symbols using Huffman coding (SDHUFF=1).
///
/// "If SDHUFF is 1, then the segment uses the Huffman encoding variant." (7.4.2.1.1)
fn decode_symbols_huffman(
    reader: &mut Reader<'_>,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    referred_tables: &[HuffmanTable],
    standard_tables: &StandardHuffmanTables,
) -> Result<Vec<DecodedRegion>> {
    // "These user-supplied Huffman decoding tables may be supplied either as a
    // Tables segment, which is referred to by the symbol dictionary segment, or
    // they may be included directly in the symbol dictionary segment, immediately
    // following the symbol dictionary segment header." (7.4.2.1.6)
    let custom_count = [
        header.flags.delta_height_table == HuffmanTableSelection::UserSupplied,
        header.flags.delta_width_table == HuffmanTableSelection::UserSupplied,
        header.flags.bitmap_size_table == HuffmanTableSelection::UserSupplied,
        header.flags.aggregate_instance_table == HuffmanTableSelection::UserSupplied,
    ]
    .into_iter()
    .filter(|x| *x)
    .count();

    if referred_tables.len() < custom_count {
        bail!(HuffmanError::MissingTables);
    }

    let mut custom_idx = 0;
    let mut get_custom = || -> &HuffmanTable {
        let table = &referred_tables[custom_idx];
        custom_idx += 1;
        table
    };

    // Select Huffman tables based on flags (7.4.2.1.6)
    // "The order of the tables that appear is in the natural order determined
    // by 7.4.2.1.1." (7.4.2.1.6)
    let mut get_table = |selection: HuffmanTableSelection| -> &HuffmanTable {
        match selection {
            HuffmanTableSelection::TableB1 => standard_tables.table_a(),
            HuffmanTableSelection::TableB2 => standard_tables.table_b(),
            HuffmanTableSelection::TableB3 => standard_tables.table_c(),
            HuffmanTableSelection::TableB4 => standard_tables.table_d(),
            HuffmanTableSelection::TableB5 => standard_tables.table_e(),
            HuffmanTableSelection::UserSupplied => get_custom(),
        }
    };

    let sdhuffdh = get_table(header.flags.delta_height_table);
    let sdhuffdw = get_table(header.flags.delta_width_table);
    let sdhuffbmsize = get_table(header.flags.bitmap_size_table);
    // TODO: Use this one.
    let _sdhuffagginst = get_table(header.flags.aggregate_instance_table);

    let num_input_symbols = input_symbols.len() as u32;
    let num_new_symbols = header.num_new_symbols;

    // "1) Create an array SDNEWSYMS of bitmaps, having SDNUMNEWSYMS entries."
    let mut new_symbols: Vec<DecodedRegion> = Vec::with_capacity(num_new_symbols as usize);

    // "2) If SDHUFF is 1 and SDREFAGG is 0, create an array SDNEWSYMWIDTHS of
    // integers, having SDNUMNEWSYMS entries."
    let mut new_sym_widths: Vec<u32> = Vec::with_capacity(num_new_symbols as usize);

    // "3) Set: HCHEIGHT = 0, NSYMSDECODED = 0"
    let mut hcheight: u32 = 0;
    let mut nsymsdecoded: u32 = 0;

    // "4) Decode each height class as follows:"
    while nsymsdecoded < num_new_symbols {
        // "b) Decode the height class delta height as described in 6.5.6.
        // Let HCDH be the decoded value."
        // "If SDHUFF is 1, decode a value using the Huffman table specified by
        // SDHUFFDH." (6.5.6)
        let hcdh = sdhuffdh
            .decode(reader)?
            .ok_or(HuffmanError::UnexpectedOob)?;

        // "Set: HCHEIGHT = HCHEIGHT + HCDH"
        hcheight = hcheight
            .checked_add_signed(hcdh)
            .ok_or(RegionError::InvalidDimension)?;

        // "SYMWIDTH = 0, TOTWIDTH = 0, HCFIRSTSYM = NSYMSDECODED"
        let mut symwidth: u32 = 0;
        let mut totwidth: u32 = 0;
        let hcfirstsym = nsymsdecoded;

        // "c) Decode each symbol within the height class as follows:"
        // "If the result of this decoding is OOB then all the symbols
        // in this height class have been decoded; proceed to step 4 d)."
        while let Some(dw) = sdhuffdw.decode(reader)? {
            // "i) Decode the delta width for the symbol as described in 6.5.7."
            // "If SDHUFF is 1, decode a value using the Huffman table specified by
            // SDHUFFDW." (6.5.7)

            // "Set: SYMWIDTH = SYMWIDTH + DW, TOTWIDTH = TOTWIDTH + SYMWIDTH"
            symwidth = symwidth
                .checked_add_signed(dw)
                .ok_or(RegionError::InvalidDimension)?;
            totwidth = totwidth
                .checked_add(symwidth)
                .ok_or(DecodeError::Overflow)?;

            if header.flags.use_refagg {
                // "ii) If SDHUFF is 0 or SDREFAGG is 1, then decode the symbol's bitmap
                // as described in 6.5.8."
                // TODO: Implement refinement/aggregate with Huffman
                bail!(DecodeError::Unsupported);
            } else {
                // "iii) If SDHUFF is 1 and SDREFAGG is 0, then set:
                // SDNEWSYMWIDTHS[NSYMSDECODED] = SYMWIDTH"
                new_sym_widths.push(symwidth);
            }

            // "iv) Set: NSYMSDECODED = NSYMSDECODED + 1"
            nsymsdecoded += 1;
        }

        // "d) If SDHUFF is 1 and SDREFAGG is 0, then decode the height class collective
        // bitmap as described in 6.5.9."
        if !header.flags.use_refagg {
            decode_height_class_collective_bitmap(
                reader,
                sdhuffbmsize,
                &mut new_symbols,
                &new_sym_widths,
                hcfirstsym,
                nsymsdecoded,
                totwidth,
                hcheight,
            )?;
        }
    }

    // "5) Determine which symbol bitmaps are exported from this symbol dictionary,
    // as described in 6.5.10."
    // "If SDHUFF is 1, decode a value using Table B.1." (6.5.10)
    let table_a = standard_tables.table_a();
    let exported = decode_exported_symbols_with(
        num_input_symbols,
        header.num_exported_symbols,
        input_symbols,
        &new_symbols,
        || Ok(table_a.decode(reader)?.ok_or(HuffmanError::UnexpectedOob)?),
    )?;

    Ok(exported)
}

/// Decode a height class collective bitmap (6.5.9).
///
/// "This field is only present if SDHUFF = 1 and SDREFAGG = 0." (6.5.9)
#[allow(clippy::too_many_arguments)]
fn decode_height_class_collective_bitmap(
    reader: &mut Reader<'_>,
    sdhuffbmsize: &HuffmanTable,
    new_symbols: &mut Vec<DecodedRegion>,
    new_sym_widths: &[u32],
    hcfirstsym: u32,
    nsymsdecoded: u32,
    totwidth: u32,
    hcheight: u32,
) -> Result<()> {
    // "1) Read the size in bytes using the SDHUFFBMSIZE Huffman table.
    // Let BMSIZE be the value decoded."
    let bmsize = sdhuffbmsize
        .decode(reader)?
        .ok_or(HuffmanError::UnexpectedOob)? as u32;

    // "2) Skip over any bits remaining in the last byte read."
    reader.align();

    // Decode the collective bitmap
    let collective_bitmap = if bmsize == 0 {
        // "3) If BMSIZE is zero, then the bitmap is stored uncompressed, and the
        // actual size in bytes is: HCHEIGHT × ⌈TOTWIDTH / 8⌉"
        let row_bytes = totwidth.div_ceil(8);

        let mut bitmap = DecodedRegion::new(totwidth, hcheight);
        for y in 0..hcheight {
            for byte_x in 0..row_bytes {
                let byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
                for bit in 0..8 {
                    let x = byte_x * 8 + bit;
                    if x < totwidth {
                        let pixel = (byte >> (7 - bit)) & 1 != 0;
                        bitmap.set_pixel(x, y, pixel);
                    }
                }
            }
        }
        bitmap
    } else {
        // "4) Otherwise, decode the bitmap using a generic bitmap decoding procedure
        // as described in 6.2. Set the parameters to this decoding procedure as
        // shown in Table 19." (MMR = 1)
        let bitmap_data = reader
            .read_bytes(bmsize as usize)
            .ok_or(ParseError::UnexpectedEof)?;

        let mut bitmap = DecodedRegion::new(totwidth, hcheight);
        decode_bitmap_mmr(&mut bitmap, bitmap_data)?;
        bitmap
    };

    // "Break up the bitmap B_HC as follows to obtain the symbols
    // SDNEWSYMS[HCFIRSTSYM] through SDNEWSYMS[NSYMSDECODED − 1]." (6.5.5, step 4d)
    //
    // "B_HC contains the NSYMSDECODED − HCFIRSTSYM symbols concatenated left-to-right,
    // with no intervening gaps."
    let mut x_offset: u32 = 0;
    for i in hcfirstsym..nsymsdecoded {
        let sym_width = new_sym_widths[i as usize];
        let mut symbol = DecodedRegion::new(sym_width, hcheight);

        // Copy pixels from collective bitmap to individual symbol
        for y in 0..hcheight {
            for x in 0..sym_width {
                let pixel = collective_bitmap.get_pixel(x_offset + x, y);
                symbol.set_pixel(x, y, pixel);
            }
        }

        new_symbols.push(symbol);
        x_offset += sym_width;
    }

    Ok(())
}

/// Determine exported symbols (6.5.10).
///
/// "The symbols that may be exported from a given dictionary include any of the
/// symbols that are input to the dictionary, plus any of the symbols defined in
/// the dictionary." (6.5.10)
///
/// The `decode_value` closure decodes the run length value:
/// - For Huffman coding (SDHUFF=1): uses Table B.1
/// - For arithmetic coding (SDHUFF=0): uses the IAEX integer decoder
fn decode_exported_symbols_with<F>(
    num_input_symbols: u32,
    num_exported: u32,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    mut decode_value: F,
) -> Result<Vec<DecodedRegion>>
where
    F: FnMut() -> Result<i32>,
{
    let num_new_symbols = new_symbols.len() as u32;
    let total_symbols = num_input_symbols + num_new_symbols;

    // "1) Set: EXINDEX = 0, CUREXFLAG = 0"
    let mut exindex: u32 = 0;
    let mut curexflag: bool = false;

    // EXFLAGS array - one bit per symbol indicating if exported
    let mut exflags = vec![false; total_symbols as usize];

    // "5) Repeat steps 2) through 4) until EXINDEX = SDNUMINSYMS + SDNUMNEWSYMS"
    while exindex < total_symbols {
        // "2) Decode a value using Table B.1 if SDHUFF is 1, or the IAEX integer
        // arithmetic decoding procedure if SDHUFF is 0. Let EXRUNLENGTH be the
        // decoded value."
        let exrunlength = decode_value()?;

        if exrunlength < 0 {
            bail!(HuffmanError::InvalidCode);
        }

        let exrunlength = exrunlength as u32;

        // "3) Set EXFLAGS[EXINDEX] through EXFLAGS[EXINDEX + EXRUNLENGTH - 1]
        // to CUREXFLAG."
        for i in 0..exrunlength {
            let idx = (exindex + i) as usize;
            if idx < exflags.len() {
                exflags[idx] = curexflag;
            }
        }

        // "4) Set: EXINDEX = EXINDEX + EXRUNLENGTH, CUREXFLAG = NOT(CUREXFLAG)"
        exindex += exrunlength;
        curexflag = !curexflag;
    }

    // "8) For each value of I from 0 to SDNUMINSYMS + SDNUMNEWSYMS - 1, if
    // EXFLAGS[I] = 1 then perform the following steps:"
    let mut exported = Vec::with_capacity(num_exported as usize);

    for (i, &is_exported) in exflags.iter().enumerate() {
        if is_exported {
            let symbol = if (i as u32) < num_input_symbols {
                // "a) If I < SDNUMINSYMS then set: SDEXSYMS[J] = SDINSYMS[I]"
                input_symbols[i].clone()
            } else {
                // "b) If I >= SDNUMINSYMS then set:
                // SDEXSYMS[J] = SDNEWSYMS[I - SDNUMINSYMS]"
                let new_idx = i - num_input_symbols as usize;
                new_symbols[new_idx].clone()
            };
            exported.push(symbol);
        }
    }

    if exported.len() != num_exported as usize {
        bail!(SymbolError::NoSymbols);
    }

    Ok(exported)
}

/// Decode symbols using direct bitmap coding (SDREFAGG=0).
fn decode_symbols_direct(
    data: &[u8],
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
) -> Result<Vec<DecodedRegion>> {
    let template = header.flags.template;
    let num_contexts = 1 << template.context_bits();
    let mut gb_contexts = vec![Context::default(); num_contexts];

    decode_symbols_with(
        data,
        header,
        input_symbols,
        |decoder, symwidth, hcheight, _| {
            decode_symbol_bitmap(decoder, &mut gb_contexts, header, symwidth, hcheight)
        },
    )
}

/// Decode symbols using refinement/aggregate coding (SDREFAGG=1).
fn decode_symbols_refagg(
    data: &[u8],
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
) -> Result<Vec<DecodedRegion>> {
    // Additional decoder for refinement (6.5.8.2)
    let mut iaai = IntegerDecoder::new(); // REFAGGNINST decoder

    // "SBSYMCODELEN: ceil(log2(SDNUMINSYMS + SDNUMNEWSYMS))" (6.5.8.2.3)
    let num_input_symbols = input_symbols.len() as u32;
    let total_symbols = num_input_symbols + header.num_new_symbols;
    let sbsymcodelen = if total_symbols <= 1 {
        1
    } else {
        32 - (total_symbols - 1).leading_zeros()
    };

    // Refinement contexts
    let gr_template = header.flags.refinement_template;
    let num_gr_contexts = 1 << gr_template.context_bits();
    let mut gr_contexts = vec![Context::default(); num_gr_contexts];

    let mut text_region_contexts = TextRegionContexts::new(sbsymcodelen);

    decode_symbols_with(
        data,
        header,
        input_symbols,
        |decoder, symwidth, hcheight, new_symbols| {
            decode_refinement_aggregate_symbol(
                decoder,
                &mut gr_contexts,
                &mut iaai,
                &mut text_region_contexts,
                header,
                input_symbols,
                new_symbols,
                symwidth,
                hcheight,
                gr_template,
            )
        },
    )
}

/// Core symbol decoding loop (6.5).
///
/// Takes a closure that decodes each individual symbol bitmap.
fn decode_symbols_with<F>(
    data: &[u8],
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    mut decode_symbol: F,
) -> Result<Vec<DecodedRegion>>
where
    F: FnMut(&mut ArithmeticDecoder<'_>, u32, u32, &[DecodedRegion]) -> Result<DecodedRegion>,
{
    let num_input_symbols = input_symbols.len() as u32;
    let num_new_symbols = header.num_new_symbols;

    // "1) Create an array SDNEWSYMS of bitmaps, having SDNUMNEWSYMS entries."
    let mut new_symbols: Vec<DecodedRegion> = Vec::with_capacity(num_new_symbols as usize);

    // Initialize arithmetic decoder and integer decoders.
    let mut arith_decoder = ArithmeticDecoder::new(data);
    let mut iadh = IntegerDecoder::new();
    let mut iadw = IntegerDecoder::new();
    let mut iaex = IntegerDecoder::new();

    // "3) Set: HCHEIGHT = 0, NSYMSDECODED = 0"
    let mut hcheight: u32 = 0;
    let mut nsymsdecoded: u32 = 0;

    // "4) Decode each height class as follows:"
    while nsymsdecoded < num_new_symbols {
        // "a) If NSYMSDECODED = SDNUMNEWSYMS then all the symbols in the
        // dictionary have been decoded; proceed to step 5)."
        // (This is checked by the while condition)

        // "b) Decode the height class delta height as described in 6.5.6.
        // Let HCDH be the decoded value."
        let hcdh = iadh
            .decode(&mut arith_decoder)
            .ok_or(SymbolError::OutOfRange)?;

        // "Set: HCHEIGHT = HCHEIGHT + HCDH"
        // HCDH can be negative, but the result must be non-negative.
        hcheight = hcheight
            .checked_add_signed(hcdh)
            .ok_or(RegionError::InvalidDimension)?;

        // "SYMWIDTH = 0, TOTWIDTH = 0, HCFIRSTSYM = NSYMSDECODED"
        let mut symwidth: u32 = 0;

        // "c) Decode each symbol within the height class as follows:"
        // "If the result of this decoding is OOB then all the symbols
        // in this height class have been decoded; proceed to step 4 d)."
        while let Some(dw) = iadw.decode(&mut arith_decoder) {
            // "i) Decode the delta width for the symbol as described in 6.5.7."

            // "Set: SYMWIDTH = SYMWIDTH + DW"
            // DW can be negative, but the result must be non-negative.
            symwidth = symwidth
                .checked_add_signed(dw)
                .ok_or(RegionError::InvalidDimension)?;

            // "ii) If SDHUFF is 0 or SDREFAGG is 1, then decode the symbol's bitmap
            // as described in 6.5.8."
            let symbol = decode_symbol(&mut arith_decoder, symwidth, hcheight, &new_symbols)?;

            // "Set: SDNEWSYMS[NSYMSDECODED] = B_S"
            new_symbols.push(symbol);

            // "iv) Set: NSYMSDECODED = NSYMSDECODED + 1"
            nsymsdecoded += 1;
        }
    }

    // "5) Determine which symbol bitmaps are exported from this symbol dictionary,
    // as described in 6.5.10."
    let exported = decode_exported_symbols_with(
        num_input_symbols,
        header.num_exported_symbols,
        input_symbols,
        &new_symbols,
        || {
            Ok(iaex
                .decode(&mut arith_decoder)
                .ok_or(SymbolError::OutOfRange)?)
        },
    )?;

    Ok(exported)
}

/// Decode a symbol bitmap using direct bitmap coding (6.5.8.1, Table 16).
///
/// "If SDREFAGG is 0, then decode the symbol's bitmap using a generic region
/// decoding procedure as described in 6.2. Set the parameters to this decoding
/// procedure as shown in Table 16."
fn decode_symbol_bitmap(
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [Context],
    header: &SymbolDictionaryHeader,
    width: u32,
    height: u32,
) -> Result<DecodedRegion> {
    // Table 16 parameters:
    // MMR = 0, GBW = SYMWIDTH, GBH = HCHEIGHT, GBTEMPLATE = SDTEMPLATE
    // TPGDON = 0, USESKIP = 0
    // GBAT = SDAT (adaptive template pixels from header)

    let mut region = DecodedRegion::new(width, height);
    let template = header.flags.template;

    // Decode each pixel using generic region decoding (6.2.5)
    // with TPGDON = 0 (no typical prediction)
    for y in 0..height {
        for x in 0..width {
            let context = gather_context(&region, x, y, template, &header.at_pixels);
            let pixel = decoder.decode(&mut contexts[context as usize]);
            region.set_pixel(x, y, pixel != 0);
        }
    }

    Ok(region)
}

/// Decode a symbol bitmap using refinement/aggregate coding (6.5.8.2).
///
/// "If SDREFAGG is 1, then the symbol's bitmap is coded by refinement and
/// aggregation of other, previously-defined, symbols." (6.5.8.2)
#[allow(clippy::too_many_arguments)]
fn decode_refinement_aggregate_symbol(
    decoder: &mut ArithmeticDecoder<'_>,
    gr_contexts: &mut [Context],
    iaai: &mut IntegerDecoder,
    text_region_contexts: &mut TextRegionContexts,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    gr_template: RefinementTemplate,
) -> Result<DecodedRegion> {
    // "1) Decode the number of symbol instances contained in the aggregation,
    // as specified in 6.5.8.2.1. Let REFAGGNINST be the value decoded." (6.5.8.2)
    let refaggninst = iaai.decode(decoder).ok_or(SymbolError::OutOfRange)?;

    if refaggninst == 1 {
        // "3) If REFAGGNINST is equal to one, then decode the bitmap as described
        // in 6.5.8.2.2." (6.5.8.2)
        // Use decoders from text_region_contexts to share context state with REFAGGNINST>1 case
        decode_single_refinement_symbol(
            decoder,
            gr_contexts,
            text_region_contexts,
            header,
            input_symbols,
            new_symbols,
            symwidth,
            hcheight,
            gr_template,
        )
    } else {
        // "2) If REFAGGNINST is greater than one, then decode the bitmap using a
        // text region decoding procedure as described in 6.4. Set the parameters
        // to this decoding procedure as shown in Table 17." (6.5.8.2)
        decode_multi_refinement_symbol(
            decoder,
            gr_contexts,
            text_region_contexts,
            header,
            input_symbols,
            new_symbols,
            symwidth,
            hcheight,
            refaggninst,
            gr_template,
        )
    }
}

/// Decode a bitmap when REFAGGNINST > 1 (6.5.8.2, Table 17).
///
/// "If there is more than one symbol in the aggregation, then the bitmap is
/// decoded using a text region decoding procedure as described in 6.4." (6.5.8.2)
#[allow(clippy::too_many_arguments)]
fn decode_multi_refinement_symbol(
    decoder: &mut ArithmeticDecoder<'_>,
    gr_contexts: &mut [Context],
    text_region_contexts: &mut TextRegionContexts,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    refaggninst: i32,
    gr_template: RefinementTemplate,
) -> Result<DecodedRegion> {
    // Build the combined symbol array SBSYMS as per 6.5.8.2.4:
    // "Set SBSYMS to an array of SDNUMINSYMS + NSYMSDECODED symbols, formed by
    // concatenating the array SDINSYMS and the first NSYMSDECODED entries of
    // the array SDNEWSYMS."
    let num_input = input_symbols.len();
    let num_new = new_symbols.len();
    let mut sbsyms: Vec<&DecodedRegion> = Vec::with_capacity(num_input + num_new);
    sbsyms.extend(input_symbols.iter().copied());
    for symbol in new_symbols {
        sbsyms.push(symbol);
    }

    // Table 17 parameters:
    // SBHUFF = SDHUFF (always 0 for our case since we don't support Huffman)
    // SBREFINE = 1
    // SBW = SYMWIDTH
    // SBH = HCHEIGHT
    // SBNUMINSTANCES = REFAGGNINST
    // SBSTRIPS = 1
    // SBNUMSYMS = SDNUMINSYMS + NSYMSDECODED
    // SBDEFPIXEL = 0
    // SBCOMBOP = OR
    // TRANSPOSED = 0
    // REFCORNER = TOPLEFT
    // SBDSOFFSET = 0
    // SBRTEMPLATE = SDRTEMPLATE
    // SBRATXn = SDRATXn, SBRATYn = SDRATYn

    let params = TextRegionParams {
        sbw: symwidth,
        sbh: hcheight,
        sbnuminstances: refaggninst as u32,
        sbstrips: 1,
        sbdefpixel: false,
        sbcombop: CombinationOperator::Or,
        transposed: false,
        refcorner: ReferenceCorner::TopLeft,
        sbdsoffset: 0,
        sbrtemplate: gr_template,
        refinement_at_pixels: &header.refinement_at_pixels,
    };

    // SBREFINE = 1 per Table 17, so we always use refinement decoding
    decode_text_region_with(
        decoder,
        &sbsyms,
        &params,
        text_region_contexts,
        |decoder, id_i, symbols, contexts| {
            // Decode R_I (refinement indicator)
            let r_i = contexts
                .iari
                .decode(decoder)
                .ok_or(SymbolError::OutOfRange)?;

            if r_i == 0 {
                Ok(SymbolBitmap::Reference(id_i))
            } else {
                let ibo_i = symbols.get(id_i).ok_or(SymbolError::OutOfRange)?;
                let wo_i = ibo_i.width;
                let ho_i = ibo_i.height;

                let rdw_i = contexts
                    .iardw
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;
                let rdh_i = contexts
                    .iardh
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;
                let rdx_i = contexts
                    .iardx
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;
                let rdy_i = contexts
                    .iardy
                    .decode(decoder)
                    .ok_or(SymbolError::OutOfRange)?;

                let grw = (wo_i as i32 + rdw_i) as u32;
                let grh = (ho_i as i32 + rdh_i) as u32;
                let grreferencedx = rdw_i.div_euclid(2) + rdx_i;
                let grreferencedy = rdh_i.div_euclid(2) + rdy_i;

                let mut refined = DecodedRegion::new(grw, grh);
                decode_bitmap(
                    decoder,
                    gr_contexts,
                    &mut refined,
                    ibo_i,
                    grreferencedx,
                    grreferencedy,
                    gr_template,
                    &header.refinement_at_pixels,
                    false,
                )?;
                Ok(SymbolBitmap::Owned(refined))
            }
        },
    )
}

/// Decode a bitmap when REFAGGNINST = 1 (6.5.8.2.2).
///
/// "If a symbol's bitmap is coded by refinement/aggregate coding, and there is
/// only one symbol in the aggregation, then the bitmap is decoded as follows."
/// (6.5.8.2.2)
#[allow(clippy::too_many_arguments)]
fn decode_single_refinement_symbol(
    decoder: &mut ArithmeticDecoder<'_>,
    gr_contexts: &mut [Context],
    text_region_contexts: &mut TextRegionContexts,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    gr_template: RefinementTemplate,
) -> Result<DecodedRegion> {
    // "2) Decode a symbol ID as described in 6.4.10, using the values of
    // SBSYMCODES and SBSYMCODELEN described in 6.5.8.2.3. Let ID_I be the
    // value decoded." (6.5.8.2.2)
    let id_i = text_region_contexts.iaid.decode(decoder) as usize;

    // "3) Decode the instance refinement X offset as described in 6.4.11.3.
    // [...] Let RDX_I be the value decoded." (6.5.8.2.2)
    let rdx_i = text_region_contexts
        .iardx
        .decode(decoder)
        .ok_or(SymbolError::OutOfRange)?;

    // "4) Decode the instance refinement Y offset as described in 6.4.11.4.
    // [...] Let RDY_I be the value decoded." (6.5.8.2.2)
    let rdy_i = text_region_contexts
        .iardy
        .decode(decoder)
        .ok_or(SymbolError::OutOfRange)?;

    // "6) Let IBO_I be SBSYMS[ID_I], where SBSYMS is as shown in 6.5.8.2.4."
    // (6.5.8.2.2)
    //
    // "Set SBSYMS to an array of SDNUMINSYMS + NSYMSDECODED symbols, formed by
    // concatenating the array SDINSYMS and the first NSYMSDECODED entries of
    // the array SDNEWSYMS." (6.5.8.2.4)
    let num_input = input_symbols.len();
    let reference = if id_i < num_input {
        input_symbols[id_i]
    } else {
        let new_idx = id_i - num_input;
        new_symbols.get(new_idx).ok_or(SymbolError::OutOfRange)?
    };

    // "The symbol's bitmap is the result of applying the generic refinement
    // region decoding procedure described in 6.3. Set the parameters to this
    // decoding procedure as shown in Table 18." (6.5.8.2.2)
    //
    // Table 18 parameters:
    // GRW = SYMWIDTH, GRH = HCHEIGHT
    // GRTEMPLATE = SDRTEMPLATE
    // GRREFERENCE = IBO_I
    // GRREFERENCEDX = RDX_I
    // GRREFERENCEDY = RDY_I
    // TPGRON = 0
    // GRATX1 = SDRATX1, GRATY1 = SDRATY1, etc.

    let mut region = DecodedRegion::new(symwidth, hcheight);

    decode_bitmap(
        decoder,
        gr_contexts,
        &mut region,
        reference,
        rdx_i,
        rdy_i,
        gr_template,
        &header.refinement_at_pixels,
        false, // TPGRON = 0
    )?;

    Ok(region)
}
