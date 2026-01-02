//! Symbol dictionary segment parsing and decoding (7.4.2, 6.5).
//!
//! This module handles parsing and decoding of symbol dictionary segments.
//! Symbol dictionaries store collections of symbol bitmaps that can be
//! referenced by text region segments.

use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext, IntegerDecoder};
use crate::bitmap::DecodedRegion;
use crate::huffman_table::{
    HuffmanResult, HuffmanTable, TABLE_A, TABLE_B, TABLE_C, TABLE_D, TABLE_E,
};
use crate::reader::Reader;
use crate::segment::generic_refinement_region::{
    GrTemplate, RefinementAdaptiveTemplatePixel, decode_refinement_bitmap_with,
};
use crate::segment::generic_region::{
    AdaptiveTemplatePixel, GbTemplate, decode_bitmap_mmr, gather_context_with_at,
};
use crate::segment::region::CombinationOperator;
use crate::segment::text_region::{
    ReferenceCorner, SymbolBitmap, TextRegionContexts, TextRegionParams, decode_text_region_with,
};

/// Huffman table selection for symbol dictionary height differences (SDHUFFDH).
///
/// "Bits 2-3: SDHUFFDH selection. This two-bit field can take on one of three
/// values, indicating which table is to be used for SDHUFFDH." (7.4.2.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SdHuffDh {
    /// "0: Table B.4"
    TableB4,
    /// "1: Table B.5"
    TableB5,
    /// "3: User-supplied table"
    UserSupplied,
}

impl SdHuffDh {
    fn from_value(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(Self::TableB4),
            1 => Ok(Self::TableB5),
            // "The value 2 is not permitted." (7.4.2.1.1)
            2 => Err("SDHUFFDH value 2 is not permitted"),
            3 => Ok(Self::UserSupplied),
            _ => unreachable!(),
        }
    }
}

/// Huffman table selection for symbol dictionary width differences (SDHUFFDW).
///
/// "Bits 4-5: SDHUFFDW selection. This two-bit field can take on one of three
/// values, indicating which table is to be used for SDHUFFDW." (7.4.2.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SdHuffDw {
    /// "0: Table B.2"
    TableB2,
    /// "1: Table B.3"
    TableB3,
    /// "3: User-supplied table"
    UserSupplied,
}

impl SdHuffDw {
    fn from_value(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(Self::TableB2),
            1 => Ok(Self::TableB3),
            // "The value 2 is not permitted." (7.4.2.1.1)
            2 => Err("SDHUFFDW value 2 is not permitted"),
            3 => Ok(Self::UserSupplied),
            _ => unreachable!(),
        }
    }
}

/// Huffman table selection for bitmap size (SDHUFFBMSIZE).
///
/// "Bit 6: SDHUFFBMSIZE selection. If this field is 0 then Table B.1 is used
/// for SDHUFFBMSIZE. If this field is 1 then a user-supplied table is used for
/// SDHUFFBMSIZE." (7.4.2.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SdHuffBmSize {
    /// Table B.1
    TableB1,
    /// User-supplied table
    UserSupplied,
}

/// Huffman table selection for aggregate instances (SDHUFFAGGINST).
///
/// "Bit 7: SDHUFFAGGINST selection. If this field is 0 then Table B.1 is used
/// for SDHUFFAGGINST. If this field is 1 then a user-supplied table is used for
/// SDHUFFAGGINST." (7.4.2.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SdHuffAggInst {
    /// Table B.1
    TableB1,
    /// User-supplied table
    UserSupplied,
}

/// Template used for refinement coding in symbol dictionary (SDRTEMPLATE).
///
/// "Bit 12: SDRTEMPLATE. This field controls the template used to decode symbol
/// bitmaps if SDREFAGG is 1. If SDREFAGG is 0, this field must contain the
/// value 0." (7.4.2.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SdRTemplate {
    /// Template 0 (13 pixels)
    Template0,
    /// Template 1 (10 pixels)
    Template1,
}

/// Parsed symbol dictionary segment flags (7.4.2.1.1).
///
/// "This two-byte field is formatted as shown in Figure 33 and as described
/// below." (7.4.2.1.1)
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionaryFlags {
    /// "Bit 0: SDHUFF. If this bit is 1, then the segment uses the Huffman
    /// encoding variant. If this bit is 0, then the segment uses the arithmetic
    /// encoding variant." (7.4.2.1.1)
    pub sdhuff: bool,

    /// "Bit 1: SDREFAGG. If this bit is 0, then no refinement or aggregate
    /// coding is used in this segment. If this bit is 1, then every symbol
    /// bitmap is refinement/aggregate coded." (7.4.2.1.1)
    pub sdrefagg: bool,

    /// "Bits 2-3: SDHUFFDH selection." (7.4.2.1.1)
    /// Only meaningful when SDHUFF is 1.
    pub sdhuffdh: SdHuffDh,

    /// "Bits 4-5: SDHUFFDW selection." (7.4.2.1.1)
    /// Only meaningful when SDHUFF is 1.
    pub sdhuffdw: SdHuffDw,

    /// "Bit 6: SDHUFFBMSIZE selection." (7.4.2.1.1)
    /// Only meaningful when SDHUFF is 1.
    pub sdhuffbmsize: SdHuffBmSize,

    /// "Bit 7: SDHUFFAGGINST selection." (7.4.2.1.1)
    /// Only meaningful when SDHUFF is 1 and SDREFAGG is 1.
    pub sdhuffagginst: SdHuffAggInst,

    /// "Bit 8: Bitmap coding context used. If SDHUFF is 1 and SDREFAGG is 0 then
    /// this field must contain the value 0." (7.4.2.1.1)
    pub _bitmap_context_used: bool,

    /// "Bit 9: Bitmap coding context retained. If SDHUFF is 1 and SDREFAGG is 0
    /// then this field must contain the value 0." (7.4.2.1.1)
    pub _bitmap_context_retained: bool,

    /// "Bits 10-11: SDTEMPLATE. This field controls the template used to decode
    /// symbol bitmaps if SDHUFF is 0. If SDHUFF is 1, this field must contain
    /// the value 0." (7.4.2.1.1)
    pub sdtemplate: GbTemplate,

    /// "Bit 12: SDRTEMPLATE. This field controls the template used to decode
    /// symbol bitmaps if SDREFAGG is 1. If SDREFAGG is 0, this field must
    /// contain the value 0." (7.4.2.1.1)
    pub sdrtemplate: SdRTemplate,
}

/// Parsed symbol dictionary segment header (7.4.2.1).
///
/// "A symbol dictionary segment's data part begins with a symbol dictionary
/// segment data header, containing the fields shown in Figure 32." (7.4.2.1)
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionaryHeader {
    /// Symbol dictionary flags (7.4.2.1.1).
    pub flags: SymbolDictionaryFlags,

    /// Symbol dictionary AT flags (7.4.2.1.2).
    ///
    /// "This field is only present if SDHUFF is 0." (7.4.2.1.2)
    /// - If SDTEMPLATE is 0: 4 AT pixels (8 bytes, Figure 34)
    /// - If SDTEMPLATE is 1, 2, or 3: 1 AT pixel (2 bytes, Figure 35)
    pub adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,

    /// Symbol dictionary refinement AT flags (7.4.2.1.3).
    ///
    /// "This field is only present if SDREFAGG is 1 and SDRTEMPLATE is 0."
    /// (7.4.2.1.3)
    /// Contains 2 AT pixels (4 bytes, Figure 36).
    pub refinement_at_pixels: Vec<RefinementAdaptiveTemplatePixel>,

    /// "SDNUMEXSYMS: This four-byte field contains the number of symbols
    /// exported from this dictionary." (7.4.2.1.4)
    pub num_exported_symbols: u32,

    /// "SDNUMNEWSYMS: This four-byte field contains the number of symbols
    /// defined in this dictionary." (7.4.2.1.5)
    pub num_new_symbols: u32,
}

/// Parse a symbol dictionary segment header (7.4.2.1).
pub(crate) fn parse_symbol_dictionary_header(
    reader: &mut Reader<'_>,
) -> Result<SymbolDictionaryHeader, &'static str> {
    // 7.4.2.1.1: Symbol dictionary flags
    let flags_word = reader.read_u16().ok_or("unexpected end of data")?;

    // "Bit 0: SDHUFF"
    let sdhuff = flags_word & 0x0001 != 0;

    // "Bit 1: SDREFAGG"
    let sdrefagg = flags_word & 0x0002 != 0;

    // "Bits 2-3: SDHUFFDH selection"
    let sdhuffdh = SdHuffDh::from_value(((flags_word >> 2) & 0x03) as u8)?;

    // "Bits 4-5: SDHUFFDW selection"
    let sdhuffdw = SdHuffDw::from_value(((flags_word >> 4) & 0x03) as u8)?;

    // "Bit 6: SDHUFFBMSIZE selection"
    let sdhuffbmsize = if flags_word & 0x0040 != 0 {
        SdHuffBmSize::UserSupplied
    } else {
        SdHuffBmSize::TableB1
    };

    // "Bit 7: SDHUFFAGGINST selection"
    let sdhuffagginst = if flags_word & 0x0080 != 0 {
        SdHuffAggInst::UserSupplied
    } else {
        SdHuffAggInst::TableB1
    };

    // "Bit 8: Bitmap coding context used"
    let bitmap_context_used = flags_word & 0x0100 != 0;

    // "Bit 9: Bitmap coding context retained"
    let bitmap_context_retained = flags_word & 0x0200 != 0;

    // "Bits 10-11: SDTEMPLATE"
    let sdtemplate = match (flags_word >> 10) & 0x03 {
        0 => GbTemplate::Template0,
        1 => GbTemplate::Template1,
        2 => GbTemplate::Template2,
        3 => GbTemplate::Template3,
        _ => unreachable!(),
    };

    // "Bit 12: SDRTEMPLATE"
    let sdrtemplate = if flags_word & 0x1000 != 0 {
        SdRTemplate::Template1
    } else {
        SdRTemplate::Template0
    };

    let flags = SymbolDictionaryFlags {
        sdhuff,
        sdrefagg,
        sdhuffdh,
        sdhuffdw,
        sdhuffbmsize,
        sdhuffagginst,
        _bitmap_context_used: bitmap_context_used,
        _bitmap_context_retained: bitmap_context_retained,
        sdtemplate,
        sdrtemplate,
    };

    // 7.4.2.1.2: Symbol dictionary AT flags
    // "This field is only present if SDHUFF is 0."
    let adaptive_template_pixels = if !sdhuff {
        parse_symbol_dictionary_at_flags(reader, sdtemplate)?
    } else {
        Vec::new()
    };

    // 7.4.2.1.3: Symbol dictionary refinement AT flags
    // "This field is only present if SDREFAGG is 1 and SDRTEMPLATE is 0."
    let refinement_at_pixels = if sdrefagg && sdrtemplate == SdRTemplate::Template0 {
        parse_symbol_dictionary_refinement_at_flags(reader)?
    } else {
        Vec::new()
    };

    // 7.4.2.1.4: SDNUMEXSYMS
    // "This four-byte field contains the number of symbols exported from this
    // dictionary."
    let num_exported_symbols = reader.read_u32().ok_or("unexpected end of data")?;

    // 7.4.2.1.5: SDNUMNEWSYMS
    // "This four-byte field contains the number of symbols defined in this
    // dictionary."
    let num_new_symbols = reader.read_u32().ok_or("unexpected end of data")?;

    Ok(SymbolDictionaryHeader {
        flags,
        adaptive_template_pixels,
        refinement_at_pixels,
        num_exported_symbols,
        num_new_symbols,
    })
}

/// Parse symbol dictionary AT flags (7.4.2.1.2).
///
/// "If SDTEMPLATE is 0, it is an eight-byte field, formatted as shown in
/// Figure 34. If SDTEMPLATE is 1, 2 or 3, it is a two-byte field formatted
/// as shown in Figure 35." (7.4.2.1.2)
fn parse_symbol_dictionary_at_flags(
    reader: &mut Reader<'_>,
    sdtemplate: GbTemplate,
) -> Result<Vec<AdaptiveTemplatePixel>, &'static str> {
    let num_pixels = match sdtemplate {
        GbTemplate::Template0 => 4,
        GbTemplate::Template1 | GbTemplate::Template2 | GbTemplate::Template3 => 1,
    };

    let mut pixels = Vec::with_capacity(num_pixels);

    for _ in 0..num_pixels {
        // "The AT coordinate X and Y fields are signed values, and may take on
        // values that are permitted according to Figure 7." (7.4.2.1.2)
        let x = reader.read_byte().ok_or("unexpected end of data")? as i8;
        let y = reader.read_byte().ok_or("unexpected end of data")? as i8;

        // Validate AT pixel location (6.2.5.4, Figure 7).
        // AT pixels must reference already-decoded pixels:
        // - y must be <= 0 (current row or above)
        // - if y == 0, x must be < 0 (strictly to the left of current pixel)
        if y > 0 || (y == 0 && x >= 0) {
            return Err("AT pixel location out of valid range");
        }

        pixels.push(AdaptiveTemplatePixel { x, y });
    }

    Ok(pixels)
}

/// Parse symbol dictionary refinement AT flags (7.4.2.1.3).
///
/// "It is a four-byte field, formatted as shown in Figure 36." (7.4.2.1.3)
fn parse_symbol_dictionary_refinement_at_flags(
    reader: &mut Reader<'_>,
) -> Result<Vec<RefinementAdaptiveTemplatePixel>, &'static str> {
    let mut pixels = Vec::with_capacity(2);

    // SDRATX1, SDRATY1
    // "The AT coordinate X and Y fields are signed values, and may take on
    // values that are permitted according to 6.3.5.3." (7.4.2.1.3)
    let x1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x1, y: y1 });

    // SDRATX2, SDRATY2
    let x2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x2, y: y2 });

    Ok(pixels)
}

/// A decoded symbol dictionary segment.
///
/// "A symbol dictionary segment is decoded according to the following steps:
/// 1) Interpret its header, as described in 7.4.2.1.
/// 2) Decode (or retrieve the results of decoding) any referred-to symbol
///    dictionary segments and tables segments"
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionary {
    /// The exported symbols (SDEXSYMS).
    /// "The symbols exported by this symbol dictionary. Contains SDNUMEXSYMS
    /// symbols." (Table 14)
    pub exported_symbols: Vec<DecodedRegion>,
}

/// Decode a symbol dictionary segment (7.4.2, 6.5).
///
/// `input_symbols` are references to symbols from referred-to symbol dictionaries
/// (SDINSYMS). Symbols are only cloned if they need to be re-exported.
///
/// `referred_tables` contains Huffman tables from referred table segments (type 53).
/// These are used when SDHUFF=1 and the Huffman flags specify user-supplied tables.
pub(crate) fn decode_symbol_dictionary(
    reader: &mut Reader<'_>,
    input_symbols: &[&DecodedRegion],
    referred_tables: &[&HuffmanTable],
) -> Result<SymbolDictionary, &'static str> {
    let header = parse_symbol_dictionary_header(reader)?;

    // "6) Invoke the symbol dictionary decoding procedure described in 6.5"
    let exported_symbols = decode_symbols(reader, &header, input_symbols, referred_tables)?;

    Ok(SymbolDictionary { exported_symbols })
}

/// Symbol dictionary decoding procedure (6.5).
///
/// "This decoding procedure is used to decode a set of symbols; these symbols
/// can then be used by text region decoding procedures, or in some cases by
/// other symbol dictionary decoding procedures." (6.5.1)
fn decode_symbols(
    reader: &mut Reader<'_>,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    referred_tables: &[&HuffmanTable],
) -> Result<Vec<DecodedRegion>, &'static str> {
    if header.flags.sdhuff {
        // "If SDHUFF is 1, then the segment uses the Huffman encoding variant."
        decode_symbols_huffman(reader, header, input_symbols, referred_tables)
    } else {
        // "If SDHUFF is 0, then the segment uses the arithmetic encoding variant."
        let data = reader.tail().ok_or("unexpected end of data")?;
        if header.flags.sdrefagg {
            decode_symbols_refagg(data, header, input_symbols)
        } else {
            decode_symbols_direct(data, header, input_symbols)
        }
    }
}

/// Decode symbols using Huffman coding (SDHUFF=1).
///
/// "If SDHUFF is 1, then the segment uses the Huffman encoding variant." (7.4.2.1.1)
fn decode_symbols_huffman(
    reader: &mut Reader<'_>,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    referred_tables: &[&HuffmanTable],
) -> Result<Vec<DecodedRegion>, &'static str> {
    // "These user-supplied Huffman decoding tables may be supplied either as a
    // Tables segment, which is referred to by the symbol dictionary segment, or
    // they may be included directly in the symbol dictionary segment, immediately
    // following the symbol dictionary segment header." (7.4.2.1.6)
    let custom_count = [
        header.flags.sdhuffdh == SdHuffDh::UserSupplied,
        header.flags.sdhuffdw == SdHuffDw::UserSupplied,
        header.flags.sdhuffbmsize == SdHuffBmSize::UserSupplied,
        header.flags.sdhuffagginst == SdHuffAggInst::UserSupplied,
    ]
    .into_iter()
    .filter(|x| *x)
    .count();

    if referred_tables.len() < custom_count {
        return Err("not enough referred huffman tables for symbol dictionary");
    }

    let mut custom_idx = 0;
    let mut get_custom = || -> Result<&HuffmanTable, &'static str> {
        let table = referred_tables[custom_idx];
        custom_idx += 1;

        Ok(table)
    };

    // Select Huffman tables based on flags (7.4.2.1.6)
    // "The order of the tables that appear is in the natural order determined
    // by 7.4.2.1.1." (7.4.2.1.6)
    let sdhuffdh: &HuffmanTable = match header.flags.sdhuffdh {
        SdHuffDh::TableB4 => &TABLE_D,
        SdHuffDh::TableB5 => &TABLE_E,
        SdHuffDh::UserSupplied => get_custom()?,
    };

    let sdhuffdw: &HuffmanTable = match header.flags.sdhuffdw {
        SdHuffDw::TableB2 => &TABLE_B,
        SdHuffDw::TableB3 => &TABLE_C,
        SdHuffDw::UserSupplied => get_custom()?,
    };

    let sdhuffbmsize: &HuffmanTable = match header.flags.sdhuffbmsize {
        SdHuffBmSize::TableB1 => &TABLE_A,
        SdHuffBmSize::UserSupplied => get_custom()?,
    };

    let _sdhuffagginst: &HuffmanTable = match header.flags.sdhuffagginst {
        SdHuffAggInst::TableB1 => &TABLE_A,
        SdHuffAggInst::UserSupplied => get_custom()?,
    };

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
        let hcdh = match sdhuffdh.decode(reader)? {
            HuffmanResult::Value(v) => v,
            HuffmanResult::OutOfBand => return Err("unexpected OOB decoding height class delta"),
        };

        // "Set: HCHEIGHT = HCHEIGHT + HCDH"
        hcheight = hcheight
            .checked_add_signed(hcdh)
            .ok_or("invalid height class height")?;

        // "SYMWIDTH = 0, TOTWIDTH = 0, HCFIRSTSYM = NSYMSDECODED"
        let mut symwidth: u32 = 0;
        let mut totwidth: u32 = 0;
        let hcfirstsym = nsymsdecoded;

        // "c) Decode each symbol within the height class as follows:"
        // "If the result of this decoding is OOB then all the symbols
        // in this height class have been decoded; proceed to step 4 d)."
        while let HuffmanResult::Value(dw) = sdhuffdw.decode(reader)? {
            // "i) Decode the delta width for the symbol as described in 6.5.7."
            // "If SDHUFF is 1, decode a value using the Huffman table specified by
            // SDHUFFDW." (6.5.7)

            // "Set: SYMWIDTH = SYMWIDTH + DW, TOTWIDTH = TOTWIDTH + SYMWIDTH"
            symwidth = symwidth
                .checked_add_signed(dw)
                .ok_or("invalid symbol width")?;
            totwidth = totwidth.checked_add(symwidth).ok_or("totwidth overflow")?;

            if header.flags.sdrefagg {
                // "ii) If SDHUFF is 0 or SDREFAGG is 1, then decode the symbol's bitmap
                // as described in 6.5.8."
                // TODO: Implement refinement/aggregate with Huffman
                return Err("SDHUFF=1 with SDREFAGG=1 not yet supported");
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
        if !header.flags.sdrefagg {
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
    let exported = decode_exported_symbols_with(
        num_input_symbols,
        header.num_exported_symbols,
        input_symbols,
        &new_symbols,
        || match TABLE_A.decode(reader)? {
            HuffmanResult::Value(v) => Ok(v),
            HuffmanResult::OutOfBand => Err("unexpected OOB decoding export flags"),
        },
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
) -> Result<(), &'static str> {
    // "1) Read the size in bytes using the SDHUFFBMSIZE Huffman table.
    // Let BMSIZE be the value decoded."
    let bmsize = match sdhuffbmsize.decode(reader)? {
        HuffmanResult::Value(v) => v as u32,
        HuffmanResult::OutOfBand => return Err("unexpected OOB decoding BMSIZE"),
    };

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
                let byte = reader.read_byte().ok_or("unexpected end of data")?;
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
            .ok_or("unexpected end of data")?;

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
) -> Result<Vec<DecodedRegion>, &'static str>
where
    F: FnMut() -> Result<i32, &'static str>,
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
            return Err("negative export run length");
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
        return Err("exported symbol count mismatch");
    }

    Ok(exported)
}

/// Decode symbols using direct bitmap coding (SDREFAGG=0).
fn decode_symbols_direct(
    data: &[u8],
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
) -> Result<Vec<DecodedRegion>, &'static str> {
    let template = header.flags.sdtemplate;
    let num_contexts = 1 << template.context_bits();
    let mut gb_contexts = vec![ArithmeticDecoderContext::default(); num_contexts];

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
) -> Result<Vec<DecodedRegion>, &'static str> {
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
    let gr_template = match header.flags.sdrtemplate {
        SdRTemplate::Template0 => GrTemplate::Template0,
        SdRTemplate::Template1 => GrTemplate::Template1,
    };
    let num_gr_contexts = match gr_template {
        GrTemplate::Template0 => 1 << 13,
        GrTemplate::Template1 => 1 << 10,
    };
    let mut gr_contexts = vec![ArithmeticDecoderContext::default(); num_gr_contexts];

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
) -> Result<Vec<DecodedRegion>, &'static str>
where
    F: FnMut(
        &mut ArithmeticDecoder<'_>,
        u32,
        u32,
        &[DecodedRegion],
    ) -> Result<DecodedRegion, &'static str>,
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
            .ok_or("unexpected OOB decoding height class delta")?;

        // "Set: HCHEIGHT = HCHEIGHT + HCDH"
        // HCDH can be negative, but the result must be non-negative.
        hcheight = hcheight
            .checked_add_signed(hcdh)
            .ok_or("invalid height class height")?;

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
                .ok_or("invalid symbol width")?;

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
            iaex.decode(&mut arith_decoder)
                .ok_or("unexpected OOB decoding export flags")
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
    contexts: &mut [ArithmeticDecoderContext],
    header: &SymbolDictionaryHeader,
    width: u32,
    height: u32,
) -> Result<DecodedRegion, &'static str> {
    // Table 16 parameters:
    // MMR = 0, GBW = SYMWIDTH, GBH = HCHEIGHT, GBTEMPLATE = SDTEMPLATE
    // TPGDON = 0, USESKIP = 0
    // GBAT = SDAT (adaptive template pixels from header)

    let mut region = DecodedRegion::new(width, height);
    let template = header.flags.sdtemplate;

    // Decode each pixel using generic region decoding (6.2.5)
    // with TPGDON = 0 (no typical prediction)
    for y in 0..height {
        for x in 0..width {
            let context =
                gather_context_with_at(&region, x, y, template, &header.adaptive_template_pixels);
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
    gr_contexts: &mut [ArithmeticDecoderContext],
    iaai: &mut IntegerDecoder,
    text_region_contexts: &mut TextRegionContexts,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    gr_template: GrTemplate,
) -> Result<DecodedRegion, &'static str> {
    // "1) Decode the number of symbol instances contained in the aggregation,
    // as specified in 6.5.8.2.1. Let REFAGGNINST be the value decoded." (6.5.8.2)
    let refaggninst = iaai
        .decode(decoder)
        .ok_or("unexpected OOB decoding REFAGGNINST")?;

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
    gr_contexts: &mut [ArithmeticDecoderContext],
    text_region_contexts: &mut TextRegionContexts,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    refaggninst: i32,
    gr_template: GrTemplate,
) -> Result<DecodedRegion, &'static str> {
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
                .ok_or("unexpected OOB decoding R_I")?;

            if r_i == 0 {
                Ok(SymbolBitmap::Reference(id_i))
            } else {
                let ibo_i = symbols.get(id_i).ok_or("symbol ID out of range")?;
                let wo_i = ibo_i.width;
                let ho_i = ibo_i.height;

                let rdw_i = contexts
                    .iardw
                    .decode(decoder)
                    .ok_or("unexpected OOB decoding RDW_I")?;
                let rdh_i = contexts
                    .iardh
                    .decode(decoder)
                    .ok_or("unexpected OOB decoding RDH_I")?;
                let rdx_i = contexts
                    .iardx
                    .decode(decoder)
                    .ok_or("unexpected OOB decoding RDX_I")?;
                let rdy_i = contexts
                    .iardy
                    .decode(decoder)
                    .ok_or("unexpected OOB decoding RDY_I")?;

                let grw = (wo_i as i32 + rdw_i) as u32;
                let grh = (ho_i as i32 + rdh_i) as u32;
                let grreferencedx = rdw_i.div_euclid(2) + rdx_i;
                let grreferencedy = rdh_i.div_euclid(2) + rdy_i;

                let mut refined = DecodedRegion::new(grw, grh);
                decode_refinement_bitmap_with(
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
    gr_contexts: &mut [ArithmeticDecoderContext],
    text_region_contexts: &mut TextRegionContexts,
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
    symwidth: u32,
    hcheight: u32,
    gr_template: GrTemplate,
) -> Result<DecodedRegion, &'static str> {
    // "2) Decode a symbol ID as described in 6.4.10, using the values of
    // SBSYMCODES and SBSYMCODELEN described in 6.5.8.2.3. Let ID_I be the
    // value decoded." (6.5.8.2.2)
    let id_i = text_region_contexts.iaid.decode(decoder) as usize;

    // "3) Decode the instance refinement X offset as described in 6.4.11.3.
    // [...] Let RDX_I be the value decoded." (6.5.8.2.2)
    let rdx_i = text_region_contexts
        .iardx
        .decode(decoder)
        .ok_or("unexpected OOB decoding RDX_I")?;

    // "4) Decode the instance refinement Y offset as described in 6.4.11.4.
    // [...] Let RDY_I be the value decoded." (6.5.8.2.2)
    let rdy_i = text_region_contexts
        .iardy
        .decode(decoder)
        .ok_or("unexpected OOB decoding RDY_I")?;

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
        new_symbols
            .get(new_idx)
            .ok_or("refinement symbol ID out of range")?
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

    decode_refinement_bitmap_with(
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
