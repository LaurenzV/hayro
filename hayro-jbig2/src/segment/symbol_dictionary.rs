//! Symbol dictionary segment parsing and decoding (7.4.2, 6.5).
//!
//! This module handles parsing and decoding of symbol dictionary segments.
//! Symbol dictionaries store collections of symbol bitmaps that can be
//! referenced by text region segments.

use crate::arithmetic_decoder::{
    ArithmeticDecoder, ArithmeticDecoderContext, IntegerDecoder, SymbolIdDecoder,
};
use crate::bitmap::DecodedRegion;
use crate::reader::Reader;
use crate::segment::generic_refinement_region::{
    GrTemplate, RefinementAdaptiveTemplatePixel, decode_refinement_bitmap_with,
};
use crate::segment::generic_region::{AdaptiveTemplatePixel, GbTemplate, gather_context_with_at};
use crate::segment::region::CombinationOperator;
use crate::segment::text_region::{ReferenceCorner, TextRegionParams, decode_text_region_refine};

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
    pub bitmap_context_used: bool,

    /// "Bit 9: Bitmap coding context retained. If SDHUFF is 1 and SDREFAGG is 0
    /// then this field must contain the value 0." (7.4.2.1.1)
    pub bitmap_context_retained: bool,

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
        bitmap_context_used,
        bitmap_context_retained,
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
    /// The parsed segment header.
    pub header: SymbolDictionaryHeader,
    /// The exported symbols (SDEXSYMS).
    /// "The symbols exported by this symbol dictionary. Contains SDNUMEXSYMS
    /// symbols." (Table 14)
    pub exported_symbols: Vec<DecodedRegion>,
}

/// Decode a symbol dictionary segment (7.4.2, 6.5).
///
/// `input_symbols` are references to symbols from referred-to symbol dictionaries
/// (SDINSYMS). Symbols are only cloned if they need to be re-exported.
pub(crate) fn decode_symbol_dictionary(
    reader: &mut Reader<'_>,
    input_symbols: &[&DecodedRegion],
) -> Result<SymbolDictionary, &'static str> {
    let header = parse_symbol_dictionary_header(reader)?;

    // Check for unsupported flags
    if header.flags.sdhuff {
        return Err("SDHUFF=1 (Huffman coding) is not supported");
    }

    let encoded_data = reader.tail().ok_or("unexpected end of data")?;

    // "6) Invoke the symbol dictionary decoding procedure described in 6.5"
    let exported_symbols = decode_symbols(encoded_data, &header, input_symbols)?;

    Ok(SymbolDictionary {
        header,
        exported_symbols,
    })
}

/// Symbol dictionary decoding procedure (6.5).
///
/// "This decoding procedure is used to decode a set of symbols; these symbols
/// can then be used by text region decoding procedures, or in some cases by
/// other symbol dictionary decoding procedures." (6.5.1)
fn decode_symbols(
    data: &[u8],
    header: &SymbolDictionaryHeader,
    input_symbols: &[&DecodedRegion],
) -> Result<Vec<DecodedRegion>, &'static str> {
    if header.flags.sdrefagg {
        decode_symbols_refagg(data, header, input_symbols)
    } else {
        decode_symbols_direct(data, header, input_symbols)
    }
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
    // Additional decoders for refinement (6.5.8.2)
    let mut iaai = IntegerDecoder::new(); // REFAGGNINST decoder
    let mut iardx = IntegerDecoder::new(); // RDX decoder
    let mut iardy = IntegerDecoder::new(); // RDY decoder

    // "SBSYMCODELEN: ceil(log2(SDNUMINSYMS + SDNUMNEWSYMS))" (6.5.8.2.3)
    let num_input_symbols = input_symbols.len() as u32;
    let total_symbols = num_input_symbols + header.num_new_symbols;
    let sbsymcodelen = if total_symbols <= 1 {
        1
    } else {
        32 - (total_symbols - 1).leading_zeros()
    };
    let mut iaid = SymbolIdDecoder::new(sbsymcodelen);

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

    decode_symbols_with(
        data,
        header,
        input_symbols,
        |decoder, symwidth, hcheight, new_symbols| {
            decode_refinement_aggregate_symbol(
                decoder,
                &mut gr_contexts,
                &mut iaai,
                &mut iaid,
                &mut iardx,
                &mut iardy,
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
        loop {
            // "i) Decode the delta width for the symbol as described in 6.5.7."
            let dw = match iadw.decode(&mut arith_decoder) {
                Some(v) => v,
                None => {
                    // "If the result of this decoding is OOB then all the symbols
                    // in this height class have been decoded; proceed to step 4 d)."
                    break;
                }
            };

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
    let exported = decode_exported_symbols(
        &mut arith_decoder,
        &mut iaex,
        num_input_symbols,
        header.num_exported_symbols,
        input_symbols,
        &new_symbols,
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
    iaid: &mut SymbolIdDecoder,
    iardx: &mut IntegerDecoder,
    iardy: &mut IntegerDecoder,
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
        decode_single_refinement_symbol(
            decoder,
            gr_contexts,
            iaid,
            iardx,
            iardy,
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
    decode_text_region_refine(decoder, &sbsyms, &params)
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
    iaid: &mut SymbolIdDecoder,
    iardx: &mut IntegerDecoder,
    iardy: &mut IntegerDecoder,
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
    let id_i = iaid.decode(decoder) as usize;

    // "3) Decode the instance refinement X offset as described in 6.4.11.3.
    // [...] Let RDX_I be the value decoded." (6.5.8.2.2)
    let rdx_i = iardx
        .decode(decoder)
        .ok_or("unexpected OOB decoding RDX_I")?;

    // "4) Decode the instance refinement Y offset as described in 6.4.11.4.
    // [...] Let RDY_I be the value decoded." (6.5.8.2.2)
    let rdy_i = iardy
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

/// Determine exported symbols (6.5.10).
///
/// "The symbols that may be exported from a given dictionary include any of the
/// symbols that are input to the dictionary, plus any of the symbols defined in
/// the dictionary." (6.5.10)
fn decode_exported_symbols(
    decoder: &mut ArithmeticDecoder<'_>,
    iaex: &mut IntegerDecoder,
    num_input_symbols: u32,
    num_exported: u32,
    input_symbols: &[&DecodedRegion],
    new_symbols: &[DecodedRegion],
) -> Result<Vec<DecodedRegion>, &'static str> {
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
        let exrunlength = iaex
            .decode(decoder)
            .ok_or("unexpected OOB decoding export flags")?;

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
