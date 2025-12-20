//! Symbol dictionary segment parsing (7.4.2).
//!
//! This module handles parsing of symbol dictionary segment headers.
//! Symbol dictionaries store collections of symbol bitmaps that can be
//! referenced by text region segments.

use crate::reader::Reader;
use crate::segment::generic_refinement_region::RefinementAdaptiveTemplatePixel;
use crate::segment::generic_region::{AdaptiveTemplatePixel, GbTemplate};

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
    
    if sdhuff {
        return Err("huffman decoding is not supported yet");
    }

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

    if bitmap_context_used || bitmap_context_retained {
        return Err("bitmap_context_used and bitmap_context_retained flags are not supported yet");
    }

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

/// A parsed symbol dictionary segment.
///
/// "A symbol dictionary segment is decoded according to the following steps:
/// 1) Interpret its header, as described in 7.4.2.1.
/// 2) Decode (or retrieve the results of decoding) any referred-to symbol
///    dictionary segments and tables segments.
/// ..." (7.4.2.2)
///
/// NOTE: Currently only header parsing is implemented. Symbol decoding will
/// be added later.
#[derive(Debug, Clone)]
pub(crate) struct SymbolDictionary {
    /// The parsed segment header.
    pub header: SymbolDictionaryHeader,
    // TODO: Add decoded symbols when decoding is implemented:
    // pub symbols: Vec<DecodedRegion>,
}

/// Parse a symbol dictionary segment (7.4.2).
///
/// "A symbol dictionary segment is decoded according to the following steps:
/// 1) Interpret its header, as described in 7.4.2.1." (7.4.2.2)
///
/// NOTE: Currently only parses the header. Full decoding will be added later.
pub(crate) fn parse_symbol_dictionary(
    reader: &mut Reader<'_>,
) -> Result<SymbolDictionary, &'static str> {
    let header = parse_symbol_dictionary_header(reader)?;

    // TODO: Implement symbol decoding procedure (6.5)
    // For now, just return the parsed header.

    Ok(SymbolDictionary { header })
}
