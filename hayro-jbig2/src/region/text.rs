//! Text region segment parsing and decoding (7.4.3, 6.4).
//!
//! "The data parts of all three of the text region segment types ('intermediate
//! text region', 'immediate text region' and 'immediate lossless text region')
//! are coded identically, but are acted upon differently, see 8.2. The syntax
//! of these segment types' data parts is specified here." (7.4.3)

use super::generic_refinement::{
    GrTemplate, RefinementAdaptiveTemplatePixel, decode_refinement_bitmap_with,
};
use super::{CombinationOperator, RegionSegmentInfo, parse_region_segment_info};
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::DecodedRegion;
use crate::huffman_table::{
    HuffmanTable, TABLE_A, TABLE_F, TABLE_G, TABLE_H, TABLE_I, TABLE_J, TABLE_K, TABLE_L, TABLE_M,
    TABLE_N, TABLE_O, TableLine,
};
use crate::integer_decoder::IntegerDecoder;
use crate::reader::Reader;

/// IAID decoder for symbol IDs (A.3).
///
/// "This decoding procedure is different from all the other integer arithmetic
/// decoding procedures. It uses fixed-length representations of the values being
/// decoded, and does not limit the number of previously-decoded bits used as
/// part of the context." (A.3)
pub(crate) struct SymbolIdDecoder {
    /// "The number of contexts required is 2^SBSYMCODELEN" (A.3)
    contexts: Vec<ArithmeticDecoderContext>,
    /// "The length is equal to SBSYMCODELEN." (A.3)
    code_len: u32,
}

impl SymbolIdDecoder {
    /// Create a new symbol ID decoder for the given code length.
    ///
    /// "The number of contexts required is 2^SBSYMCODELEN, which is less than
    /// twice the maximum symbol ID." (A.3)
    pub(crate) fn new(code_len: u32) -> Self {
        let num_contexts = 1_usize << code_len;
        Self {
            contexts: vec![ArithmeticDecoderContext::default(); num_contexts],
            code_len,
        }
    }

    /// Decode a symbol ID.
    ///
    /// "The procedure for decoding an integer using the IAID decoding procedure
    /// is as follows:" (A.3)
    pub(crate) fn decode(&mut self, decoder: &mut ArithmeticDecoder<'_>) -> u32 {
        // "1) Set: PREV = 1" (A.3)
        let mut prev = 1_u32;

        // "2) Decode SBSYMCODELEN bits as follows:" (A.3)
        for _ in 0..self.code_len {
            // "a) Decode a bit with CX equal to 'IAID + PREV' where '+' represents
            // concatenation, and the rightmost SBSYMCODELEN + 1 bits of PREV are
            // used." (A.3)
            let ctx_mask = (1_u32 << (self.code_len + 1)) - 1;
            let ctx_idx = (prev & ctx_mask) as usize;
            let d = decoder.decode(&mut self.contexts[ctx_idx]);

            // "b) After each bit is decoded, set: PREV = (PREV << 1) OR D
            // where D represents the value of the just-decoded bit." (A.3)
            prev = (prev << 1) | d;
        }

        // "3) After SBSYMCODELEN bits have been decoded, set:
        //     PREV = PREV - 2^SBSYMCODELEN
        // This step has the effect of clearing the topmost (leading 1) bit of
        // PREV before returning it." (A.3)
        prev -= 1 << self.code_len;

        // "4) The contents of PREV are the result of this invocation of the IAID
        // decoding procedure." (A.3)
        prev
    }
}

/// Shared integer decoder contexts for text region decoding.
pub(crate) struct TextRegionContexts {
    /// IADT: Strip delta T decoder (6.4.6)
    pub iadt: IntegerDecoder,
    /// IAFS: First symbol S coordinate decoder (6.4.7)
    pub iafs: IntegerDecoder,
    /// IADS: Subsequent symbol S coordinate decoder (6.4.8)
    pub iads: IntegerDecoder,
    /// IAIT: Symbol instance T coordinate decoder (6.4.9)
    pub iait: IntegerDecoder,
    /// IAID: Symbol ID decoder (6.4.10)
    pub iaid: SymbolIdDecoder,
    /// IARI: Refinement image indicator decoder (6.4.11)
    pub iari: IntegerDecoder,
    /// IARDW: Refinement delta width decoder (6.4.11.1)
    pub iardw: IntegerDecoder,
    /// IARDH: Refinement delta height decoder (6.4.11.2)
    pub iardh: IntegerDecoder,
    /// IARDX: Refinement X offset decoder (6.4.11.3)
    pub iardx: IntegerDecoder,
    /// IARDY: Refinement Y offset decoder (6.4.11.4)
    pub iardy: IntegerDecoder,
}

impl TextRegionContexts {
    /// Create new text region contexts with the given symbol code length.
    pub(crate) fn new(sbsymcodelen: u32) -> Self {
        Self {
            iadt: IntegerDecoder::new(),
            iafs: IntegerDecoder::new(),
            iads: IntegerDecoder::new(),
            iait: IntegerDecoder::new(),
            iaid: SymbolIdDecoder::new(sbsymcodelen),
            iari: IntegerDecoder::new(),
            iardw: IntegerDecoder::new(),
            iardh: IntegerDecoder::new(),
            iardx: IntegerDecoder::new(),
            iardy: IntegerDecoder::new(),
        }
    }
}

/// Reference corner for symbol placement (REFCORNER).
///
/// "Bits 4-5: REFCORNER. The four values that this two-bit field can take are:
/// 0 BOTTOMLEFT
/// 1 TOPLEFT
/// 2 BOTTOMRIGHT
/// 3 TOPRIGHT" (7.4.3.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceCorner {
    /// "0 BOTTOMLEFT"
    BottomLeft,
    /// "1 TOPLEFT"
    TopLeft,
    /// "2 BOTTOMRIGHT"
    BottomRight,
    /// "3 TOPRIGHT"
    TopRight,
}

impl ReferenceCorner {
    fn from_value(value: u8) -> Self {
        match value {
            0 => Self::BottomLeft,
            1 => Self::TopLeft,
            2 => Self::BottomRight,
            3 => Self::TopRight,
            _ => unreachable!(),
        }
    }
}

/// Parsed text region segment flags (7.4.3.1.1).
///
/// "This two-byte field is formatted as shown in Figure 38 and as described
/// below." (7.4.3.1.1)
#[derive(Debug, Clone)]
pub(crate) struct TextRegionFlags {
    /// "Bit 0: SBHUFF. If this bit is 1, then the segment uses the Huffman
    /// encoding variant. If this bit is 0, then the segment uses the arithmetic
    /// encoding variant. The setting of this flag determines how the data in
    /// this segment are encoded." (7.4.3.1.1)
    pub sbhuff: bool,

    /// "Bit 1: SBREFINE. If this bit is 0, then the segment contains no symbol
    /// instance refinements. If this bit is 1, then the segment may contain
    /// symbol instance refinements." (7.4.3.1.1)
    pub sbrefine: bool,

    /// "Bits 2-3: LOGSBSTRIPS. This two-bit field codes the base-2 logarithm of
    /// the strip size used to encode the segment. Thus, strip sizes of 1, 2, 4,
    /// and 8 can be encoded." (7.4.3.1.1)
    pub log_sb_strips: u8,

    /// "Bits 4-5: REFCORNER." (7.4.3.1.1)
    pub reference_corner: ReferenceCorner,

    /// "Bit 6: TRANSPOSED. If this bit is 1, then the primary direction of
    /// coding is top-to-bottom. If this bit is 0, then the primary direction
    /// of coding is left-to-right. This allows for text running up and down
    /// the page." (7.4.3.1.1)
    pub transposed: bool,

    /// "Bits 7-8: SBCOMBOP. This field has four possible values, representing
    /// one of four possible combination operators:
    /// 0 OR
    /// 1 AND
    /// 2 XOR
    /// 3 XNOR" (7.4.3.1.1)
    pub combination_operator: CombinationOperator,

    /// "Bit 9: SBDEFPIXEL. This bit contains the initial value for every pixel
    /// in the text region, before any symbols are drawn." (7.4.3.1.1)
    pub default_pixel: bool,

    /// "Bits 10-14: SBDSOFFSET. This signed five-bit field contains the value
    /// of SBDSOFFSET – see 6.4.8." (7.4.3.1.1)
    pub ds_offset: i8,

    /// "Bit 15: SBRTEMPLATE. This field controls the template used to decode
    /// symbol instance refinements if SBREFINE is 1. If SBREFINE is 0, this
    /// field must contain the value 0." (7.4.3.1.1)
    pub sbrtemplate: u8,
}

/// Text region segment Huffman flags (7.4.3.1.2).
///
/// "This field is only present if SBHUFF is 1. This two-byte field is formatted
/// as shown in Figure 39 and as described below." (7.4.3.1.2)
#[derive(Debug, Clone)]
pub(crate) struct TextRegionHuffmanFlags {
    /// "Bits 0-1: SBHUFFFS selection. This two-bit field can take on one of
    /// three values, indicating which table is to be used for SBHUFFFS.
    /// 0 Table B.6
    /// 1 Table B.7
    /// 3 User-supplied table
    /// The value 2 is not permitted." (7.4.3.1.2)
    pub sbhufffs: u8,

    /// "Bits 2-3: SBHUFFDS selection. This two-bit field can take on one of
    /// four values, indicating which table is to be used for SBHUFFDS.
    /// 0 Table B.8
    /// 1 Table B.9
    /// 2 Table B.10
    /// 3 User-supplied table" (7.4.3.1.2)
    pub sbhuffds: u8,

    /// "Bits 4-5: SBHUFFDT selection. This two-bit field can take on one of
    /// four values, indicating which table is to be used for SBHUFFDT.
    /// 0 Table B.11
    /// 1 Table B.12
    /// 2 Table B.13
    /// 3 User-supplied table" (7.4.3.1.2)
    pub sbhuffdt: u8,

    /// "Bits 6-7: SBHUFFRDW selection. This two-bit field can take on one of
    /// three values, indicating which table is to be used for SBHUFFRDW.
    /// 0 Table B.14
    /// 1 Table B.15
    /// 3 User-supplied table
    /// The value 2 is not permitted. If SBREFINE is 0 then this field must
    /// contain the value 0." (7.4.3.1.2)
    pub sbhuffrdw: u8,

    /// "Bits 8-9: SBHUFFRDH selection." (7.4.3.1.2)
    pub sbhuffrdh: u8,

    /// "Bits 10-11: SBHUFFRDY selection." (7.4.3.1.2)
    pub sbhuffrdy: u8,

    /// "Bits 12-13: SBHUFFRDX selection." (7.4.3.1.2)
    pub sbhuffrdx: u8,

    /// "Bit 14: SBHUFFRSIZE selection. If this field is 0 then Table B.1 is
    /// used for SBHUFFRSIZE. If this field is 1 then a user-supplied table is
    /// used for SBHUFFRSIZE. If SBREFINE is 0 then this field must contain
    /// the value 0." (7.4.3.1.2)
    pub sbhuffrsize: u8,
}

/// Parsed text region segment header (7.4.3.1).
///
/// "The data part of a text region segment begins with a text region segment
/// data header. This header contains the fields shown in Figure 37 and
/// described below." (7.4.3.1)
#[derive(Debug, Clone)]
pub(crate) struct TextRegionHeader {
    /// "Region segment information field – see 7.4.1." (7.4.3.1)
    pub region_info: RegionSegmentInfo,

    /// "Text region segment flags – see 7.4.3.1.1." (7.4.3.1)
    pub flags: TextRegionFlags,

    /// "Text region segment Huffman flags – see 7.4.3.1.2." (7.4.3.1)
    /// "This field is only present if SBHUFF is 1."
    pub huffman_flags: Option<TextRegionHuffmanFlags>,

    /// "Text region segment refinement AT flags – see 7.4.3.1.3." (7.4.3.1)
    /// "This field is only present if SBREFINE is 1 and SBRTEMPLATE is 0."
    /// Contains 2 AT pixels (4 bytes, Figure 40).
    pub refinement_at_pixels: Vec<RefinementAdaptiveTemplatePixel>,

    /// "SBNUMINSTANCES – see 7.4.3.1.4." (7.4.3.1)
    /// "This four-byte field contains the number of symbol instances coded in
    /// this segment." (7.4.3.1.4)
    pub num_instances: u32,
}

/// Parse text region segment flags (7.4.3.1.1).
fn parse_text_region_flags(reader: &mut Reader<'_>) -> Result<TextRegionFlags, &'static str> {
    let flags_word = reader.read_u16().ok_or("unexpected end of data")?;

    // "Bit 0: SBHUFF"
    let sbhuff = flags_word & 0x0001 != 0;

    // "Bit 1: SBREFINE"
    let sbrefine = flags_word & 0x0002 != 0;

    // "Bits 2-3: LOGSBSTRIPS"
    let log_sb_strips = ((flags_word >> 2) & 0x03) as u8;

    // "Bits 4-5: REFCORNER"
    let reference_corner = ReferenceCorner::from_value(((flags_word >> 4) & 0x03) as u8);

    // "Bit 6: TRANSPOSED"
    let transposed = flags_word & 0x0040 != 0;

    // "Bits 7-8: SBCOMBOP"
    let sbcombop_value = ((flags_word >> 7) & 0x03) as u8;
    let combination_operator = match sbcombop_value {
        0 => CombinationOperator::Or,
        1 => CombinationOperator::And,
        2 => CombinationOperator::Xor,
        3 => CombinationOperator::Xnor,
        _ => unreachable!(),
    };

    // "Bit 9: SBDEFPIXEL"
    let default_pixel = flags_word & 0x0200 != 0;

    // "Bits 10-14: SBDSOFFSET" (signed 5-bit field)
    let ds_offset_raw = ((flags_word >> 10) & 0x1F) as u8;
    // Sign-extend from 5 bits to i8
    let ds_offset = if ds_offset_raw & 0x10 != 0 {
        // Negative value: sign extend
        (ds_offset_raw | 0xE0) as i8
    } else {
        ds_offset_raw as i8
    };

    // "Bit 15: SBRTEMPLATE"
    let sbrtemplate = ((flags_word >> 15) & 0x01) as u8;

    Ok(TextRegionFlags {
        sbhuff,
        sbrefine,
        log_sb_strips,
        reference_corner,
        transposed,
        combination_operator,
        default_pixel,
        ds_offset,
        sbrtemplate,
    })
}

/// Parse text region refinement AT flags (7.4.3.1.3).
///
/// "This field is only present if SBREFINE is 1 and SBRTEMPLATE is 0. It is a
/// four-byte field, formatted as shown in Figure 40 and as described below."
/// (7.4.3.1.3)
fn parse_text_region_refinement_at_flags(
    reader: &mut Reader<'_>,
) -> Result<Vec<RefinementAdaptiveTemplatePixel>, &'static str> {
    let mut pixels = Vec::with_capacity(2);

    // "Byte 0: SBRATX1"
    // "Byte 1: SBRATY1"
    // "The AT coordinate X and Y fields are signed values, and may take on
    // values that are permitted according to 6.3.5.3." (7.4.3.1.3)
    let x1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x1, y: y1 });

    // "Byte 2: SBRATX2"
    // "Byte 3: SBRATY2"
    let x2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x2, y: y2 });

    Ok(pixels)
}

/// Parse text region Huffman flags (7.4.3.1.2).
fn parse_text_region_huffman_flags(
    reader: &mut Reader<'_>,
) -> Result<TextRegionHuffmanFlags, &'static str> {
    let flags_word = reader.read_u16().ok_or("unexpected end of data")?;

    // "Bits 0-1: SBHUFFFS selection"
    let sbhufffs = (flags_word & 0x03) as u8;

    // "Bits 2-3: SBHUFFDS selection"
    let sbhuffds = ((flags_word >> 2) & 0x03) as u8;

    // "Bits 4-5: SBHUFFDT selection"
    let sbhuffdt = ((flags_word >> 4) & 0x03) as u8;

    // "Bits 6-7: SBHUFFRDW selection"
    let sbhuffrdw = ((flags_word >> 6) & 0x03) as u8;

    // "Bits 8-9: SBHUFFRDH selection"
    let sbhuffrdh = ((flags_word >> 8) & 0x03) as u8;

    // "Bits 10-11: SBHUFFRDY selection"
    let sbhuffrdy = ((flags_word >> 10) & 0x03) as u8;

    // "Bits 12-13: SBHUFFRDX selection"
    let sbhuffrdx = ((flags_word >> 12) & 0x03) as u8;

    // "Bit 14: SBHUFFRSIZE selection"
    let sbhuffrsize = ((flags_word >> 14) & 0x01) as u8;

    Ok(TextRegionHuffmanFlags {
        sbhufffs,
        sbhuffds,
        sbhuffdt,
        sbhuffrdw,
        sbhuffrdh,
        sbhuffrdy,
        sbhuffrdx,
        sbhuffrsize,
    })
}

/// Parse a text region segment header (7.4.3.1).
pub(crate) fn parse_text_region_header(
    reader: &mut Reader<'_>,
) -> Result<TextRegionHeader, &'static str> {
    // "Region segment information field – see 7.4.1."
    let region_info = parse_region_segment_info(reader)?;

    // "Text region segment flags – see 7.4.3.1.1."
    let flags = parse_text_region_flags(reader)?;

    // "Text region segment Huffman flags – see 7.4.3.1.2."
    // "This field is only present if SBHUFF is 1."
    let huffman_flags = if flags.sbhuff {
        Some(parse_text_region_huffman_flags(reader)?)
    } else {
        None
    };

    // "Text region segment refinement AT flags – see 7.4.3.1.3."
    // "This field is only present if SBREFINE is 1 and SBRTEMPLATE is 0."
    let refinement_at_pixels = if flags.sbrefine && flags.sbrtemplate == 0 {
        parse_text_region_refinement_at_flags(reader)?
    } else {
        Vec::new()
    };

    // "SBNUMINSTANCES – see 7.4.3.1.4."
    // "This four-byte field contains the number of symbol instances coded in
    // this segment."
    let num_instances = reader.read_u32().ok_or("unexpected end of data")?;

    Ok(TextRegionHeader {
        region_info,
        flags,
        huffman_flags,
        refinement_at_pixels,
        num_instances,
    })
}

/// Parameters for text region decoding.
///
/// This can be constructed from a `TextRegionHeader` or with explicit values
/// (e.g., for Table 17 aggregated symbol decoding).
pub(crate) struct TextRegionParams<'a> {
    /// SBW: Region width.
    pub sbw: u32,
    /// SBH: Region height.
    pub sbh: u32,
    /// SBNUMINSTANCES: Number of symbol instances.
    pub sbnuminstances: u32,
    /// SBSTRIPS: Strip size.
    pub sbstrips: u32,
    /// SBDEFPIXEL: Default pixel value.
    pub sbdefpixel: bool,
    /// SBCOMBOP: Combination operator.
    pub sbcombop: CombinationOperator,
    /// TRANSPOSED: Transposed flag.
    pub transposed: bool,
    /// REFCORNER: Reference corner.
    pub refcorner: ReferenceCorner,
    /// SBDSOFFSET: S offset.
    pub sbdsoffset: i32,
    /// SBRTEMPLATE: Refinement template.
    pub sbrtemplate: GrTemplate,
    /// SBRATXn/SBRATYn: Refinement AT pixels.
    pub refinement_at_pixels: &'a [RefinementAdaptiveTemplatePixel],
}

impl<'a> TextRegionParams<'a> {
    /// Create parameters from a parsed text region header.
    pub(crate) fn from_header(header: &'a TextRegionHeader) -> Self {
        let sbrtemplate = if header.flags.sbrtemplate == 0 {
            GrTemplate::Template0
        } else {
            GrTemplate::Template1
        };

        Self {
            sbw: header.region_info.width,
            sbh: header.region_info.height,
            sbnuminstances: header.num_instances,
            sbstrips: 1_u32 << header.flags.log_sb_strips,
            sbdefpixel: header.flags.default_pixel,
            sbcombop: header.flags.combination_operator,
            transposed: header.flags.transposed,
            refcorner: header.flags.reference_corner,
            sbdsoffset: header.flags.ds_offset as i32,
            sbrtemplate,
            refinement_at_pixels: &header.refinement_at_pixels,
        }
    }
}

/// Decode a text region segment (6.4).
///
/// "This decoding procedure is used to decode a bitmap by decoding a number of
/// symbol instances. A symbol instance contains a location and a symbol ID, and
/// possibly a refinement bitmap. These symbol instances are combined to form
/// the decoded bitmap." (6.4.1)
///
/// The `referred_tables` parameter contains Huffman tables from referred table
/// segments (type 53). These are used when SBHUFF=1 and the Huffman flags
/// specify user-supplied tables.
pub(crate) fn decode_text_region(
    reader: &mut Reader<'_>,
    symbols: &[&DecodedRegion],
    referred_tables: &[&HuffmanTable],
) -> Result<DecodedRegion, &'static str> {
    let header = parse_text_region_header(reader)?;
    let params = TextRegionParams::from_header(&header);

    let mut sbreg = if header.flags.sbhuff {
        // "If this bit is 1, then the segment uses the Huffman encoding variant."
        // (7.4.3.1.1)
        decode_text_region_huffman(reader, symbols, &header, &params, referred_tables)?
    } else {
        // "If this bit is 0, then the segment uses the arithmetic encoding variant."
        // (7.4.3.1.1)
        let data = reader.tail().ok_or("unexpected end of data")?;
        let mut decoder = ArithmeticDecoder::new(data);

        if header.flags.sbrefine {
            decode_text_region_refine(&mut decoder, symbols, &params)?
        } else {
            decode_text_region_direct(&mut decoder, symbols, &params)?
        }
    };

    // Set location info from header
    sbreg.x_location = header.region_info.x_location;
    sbreg.y_location = header.region_info.y_location;
    sbreg.combination_operator = header.region_info.combination_operator;

    Ok(sbreg)
}

/// Decode text region without refinement (SBREFINE=0).
fn decode_text_region_direct(
    decoder: &mut ArithmeticDecoder<'_>,
    symbols: &[&DecodedRegion],
    params: &TextRegionParams<'_>,
) -> Result<DecodedRegion, &'static str> {
    let sbnumsyms = symbols.len() as u32;
    if sbnumsyms == 0 {
        return Err("text region has no symbols");
    }
    let sbsymcodelen = 32 - (sbnumsyms - 1).leading_zeros();
    let mut contexts = TextRegionContexts::new(sbsymcodelen);

    decode_text_region_with(
        decoder,
        symbols,
        params,
        &mut contexts,
        |_decoder, id_i, _symbols, _contexts| {
            // "If SBREFINE is 0, then set R_I to 0." (6.4.11)
            // "If R_I is 0 then set the symbol instance bitmap IB_I to SBSYMS[ID_I]."
            Ok(SymbolBitmap::Reference(id_i))
        },
    )
}

/// Decode text region with refinement (SBREFINE=1).
///
/// This is also used for aggregated symbol decoding (REFAGGNINST > 1)
/// per Table 17, which always uses SBREFINE=1.
pub(crate) fn decode_text_region_refine(
    decoder: &mut ArithmeticDecoder<'_>,
    symbols: &[&DecodedRegion],
    params: &TextRegionParams<'_>,
) -> Result<DecodedRegion, &'static str> {
    // Create fresh contexts (for normal text region segments)
    let sbnumsyms = symbols.len() as u32;
    if sbnumsyms == 0 {
        return Err("text region has no symbols");
    }
    let sbsymcodelen = 32 - (sbnumsyms - 1).leading_zeros();
    let mut contexts = TextRegionContexts::new(sbsymcodelen);

    // Create refinement contexts
    let num_gr_contexts = match params.sbrtemplate {
        GrTemplate::Template0 => 1 << 13,
        GrTemplate::Template1 => 1 << 10,
    };
    let mut gr_contexts = vec![ArithmeticDecoderContext::default(); num_gr_contexts];

    decode_text_region_with(
        decoder,
        symbols,
        params,
        &mut contexts,
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
                    &mut gr_contexts,
                    &mut refined,
                    ibo_i,
                    grreferencedx,
                    grreferencedy,
                    params.sbrtemplate,
                    params.refinement_at_pixels,
                    false,
                )?;
                Ok(SymbolBitmap::Owned(refined))
            }
        },
    )
}

/// Result of determining a symbol instance bitmap.
pub(crate) enum SymbolBitmap {
    /// Use the symbol at this index directly (`R_I` = 0).
    Reference(usize),
    /// Use this refined bitmap (`R_I` = 1).
    Owned(DecodedRegion),
}

/// Core text region decoding loop (6.4.5).
///
/// Takes a closure that determines each symbol instance bitmap.
pub(crate) fn decode_text_region_with<F>(
    decoder: &mut ArithmeticDecoder<'_>,
    symbols: &[&DecodedRegion],
    params: &TextRegionParams<'_>,
    contexts: &mut TextRegionContexts,
    mut get_symbol_bitmap: F,
) -> Result<DecodedRegion, &'static str>
where
    F: FnMut(
        &mut ArithmeticDecoder<'_>,
        usize,
        &[&DecodedRegion],
        &mut TextRegionContexts,
    ) -> Result<SymbolBitmap, &'static str>,
{
    let sbw = params.sbw;
    let sbh = params.sbh;
    let sbnuminstances = params.sbnuminstances;
    let sbstrips = params.sbstrips;
    let sbdefpixel = params.sbdefpixel;
    let transposed = params.transposed;
    let refcorner = params.refcorner;
    let sbdsoffset = params.sbdsoffset;
    let sbcombop = params.sbcombop;

    // "1) Fill a bitmap SBREG, of the size given by SBW and SBH, with the
    // SBDEFPIXEL value." (6.4.5)
    let mut sbreg = DecodedRegion::new(sbw, sbh);

    if sbdefpixel {
        for pixel in &mut sbreg.data {
            *pixel = true;
        }
    }

    // "2) Decode the initial STRIPT value as described in 6.4.6. Negate the
    // decoded value and assign this negated value to the variable STRIPT.
    // Assign the value 0 to FIRSTS. Assign the value 0 to NINSTANCES." (6.4.5)
    let initial_stript = decode_strip_delta_t(decoder, &mut contexts.iadt, sbstrips)?;
    let mut stript: i32 = -initial_stript;
    let mut firsts: i32 = 0;
    let mut ninstances: u32 = 0;

    // "4) Decode each strip as follows:" (6.4.5)
    while ninstances < sbnuminstances {
        // "a) If NINSTANCES is equal to SBNUMINSTANCES then there are no more
        // strips to decode, and the process of decoding the text region is
        // complete; proceed to step 5)." (6.4.5)
        // (checked by while condition)

        // "b) Decode the strip's delta T value as described in 6.4.6. Let DT be
        // the decoded value. Set: STRIPT = STRIPT + DT" (6.4.5)
        let dt = decode_strip_delta_t(decoder, &mut contexts.iadt, sbstrips)?;
        stript += dt;

        // "c) Decode each symbol instance in the strip as follows:" (6.4.5)
        let mut first_symbol_in_strip = true;
        let mut curs: i32 = 0;

        loop {
            // "i) If the current symbol instance is the first symbol instance in
            // the strip, then decode the first symbol instance's S coordinate as
            // described in 6.4.7. Let DFS be the decoded value. Set:
            //     FIRSTS = FIRSTS + DFS
            //     CURS = FIRSTS" (6.4.5)
            if first_symbol_in_strip {
                let dfs = contexts
                    .iafs
                    .decode(decoder)
                    .ok_or("unexpected OOB decoding first S coordinate")?;
                firsts += dfs;
                curs = firsts;
                first_symbol_in_strip = false;
            } else {
                // "ii) Otherwise, if the current symbol instance is not the first
                // symbol instance in the strip, decode the symbol instance's S
                // coordinate as described in 6.4.8. If the result of this decoding
                // is OOB then the last symbol instance of the strip has been decoded;
                // proceed to step 3 d). Otherwise, let IDS be the decoded value. Set:
                //     CURS = CURS + IDS + SBDSOFFSET" (6.4.5)
                match contexts.iads.decode(decoder) {
                    Some(ids) => {
                        curs = curs + ids + sbdsoffset;
                    }
                    None => {
                        // OOB - end of strip
                        break;
                    }
                }
            }

            // "iii) Decode the symbol instance's T coordinate as described in 6.4.9.
            // Let CURT be the decoded value. Set: T_I = STRIPT + CURT" (6.4.5)
            let curt = decode_symbol_t_coordinate(decoder, &mut contexts.iait, sbstrips)?;
            let t_i = stript + curt;

            // "iv) Decode the symbol instance's symbol ID as described in 6.4.10.
            // Let ID_I be the decoded value." (6.4.5)
            let id_i = contexts.iaid.decode(decoder) as usize;

            // "v) Determine the symbol instance's bitmap IB_I as described in 6.4.11.
            // The width and height of this bitmap shall be denoted as W_I and H_I
            // respectively." (6.4.5)
            let symbol_bitmap = get_symbol_bitmap(decoder, id_i, symbols, contexts)?;
            let (ib_i, w_i, h_i): (&DecodedRegion, i32, i32) = match &symbol_bitmap {
                SymbolBitmap::Reference(idx) => {
                    let sym = symbols.get(*idx).ok_or("symbol ID out of range")?;
                    (sym, sym.width as i32, sym.height as i32)
                }
                SymbolBitmap::Owned(region) => (region, region.width as i32, region.height as i32),
            };

            // "vi) Update CURS as follows:" (6.4.5)
            // - If TRANSPOSED is 0, and REFCORNER is TOPRIGHT or BOTTOMRIGHT, set:
            //     CURS = CURS + W_I - 1
            // - If TRANSPOSED is 1, and REFCORNER is BOTTOMLEFT or BOTTOMRIGHT, set:
            //     CURS = CURS + H_I - 1
            // - Otherwise, do not change CURS in this step.
            if !transposed
                && (refcorner == ReferenceCorner::TopRight
                    || refcorner == ReferenceCorner::BottomRight)
            {
                curs += w_i - 1;
            } else if transposed
                && (refcorner == ReferenceCorner::BottomLeft
                    || refcorner == ReferenceCorner::BottomRight)
            {
                curs += h_i - 1;
            }

            // "vii) Set: S_I = CURS" (6.4.5)
            let s_i = curs;

            // "viii) Determine the location of the symbol instance bitmap with
            // respect to SBREG as follows:" (6.4.5)
            let (x, y) = compute_symbol_location(s_i, t_i, w_i, h_i, transposed, refcorner);

            // "x) Draw IB_I into SBREG. Combine each pixel of IB_I with the current
            // value of the corresponding pixel in SBREG, using the combination
            // operator specified by SBCOMBOP. Write the results of each combination
            // into that pixel in SBREG." (6.4.5)
            draw_symbol(&mut sbreg, ib_i, x, y, sbcombop);

            // "xi) Update CURS as follows:" (6.4.5)
            // - If TRANSPOSED is 0, and REFCORNER is TOPLEFT or BOTTOMLEFT, set:
            //     CURS = CURS + W_I - 1
            // - If TRANSPOSED is 1, and REFCORNER is TOPLEFT or TOPRIGHT, set:
            //     CURS = CURS + H_I - 1
            // - Otherwise, do not change CURS in this step.
            if !transposed
                && (refcorner == ReferenceCorner::TopLeft
                    || refcorner == ReferenceCorner::BottomLeft)
            {
                curs += w_i - 1;
            } else if transposed
                && (refcorner == ReferenceCorner::TopLeft || refcorner == ReferenceCorner::TopRight)
            {
                curs += h_i - 1;
            }

            // "xii) Set: NINSTANCES = NINSTANCES + 1" (6.4.5)
            ninstances += 1;
        }
    }

    // "5) After all the strips have been decoded, the current contents of SBREG
    // are the results that shall be obtained by every decoder" (6.4.5)
    Ok(sbreg)
}

/// Decode strip delta T (6.4.6).
///
/// "If SBHUFF is 0, decode a value using the IADT integer arithmetic decoding
/// procedure (see Annex A) and multiply the resulting value by SBSTRIPS." (6.4.6)
fn decode_strip_delta_t(
    decoder: &mut ArithmeticDecoder<'_>,
    iadt: &mut IntegerDecoder,
    sbstrips: u32,
) -> Result<i32, &'static str> {
    let value = iadt
        .decode(decoder)
        .ok_or("unexpected OOB decoding strip delta T")?;
    Ok(value * sbstrips as i32)
}

/// Decode symbol instance T coordinate (6.4.9).
///
/// "If SBSTRIPS = 1, then the value decoded is always zero." (6.4.9)
/// "If SBHUFF is 0, decode a value using the IAIT integer arithmetic decoding
/// procedure (see Annex A)." (6.4.9)
fn decode_symbol_t_coordinate(
    decoder: &mut ArithmeticDecoder<'_>,
    iait: &mut IntegerDecoder,
    sbstrips: u32,
) -> Result<i32, &'static str> {
    if sbstrips == 1 {
        // "NOTE – If SBSTRIPS = 1, then no bits are consumed, and the IAIT
        // integer arithmetic decoding procedure is never invoked." (6.4.9)
        Ok(0)
    } else {
        let value = iait
            .decode(decoder)
            .ok_or("unexpected OOB decoding symbol T coordinate")?;
        Ok(value)
    }
}

/// Compute the location of a symbol instance bitmap (6.4.5 step viii).
///
/// Returns (x, y) coordinates where the symbol should be placed.
fn compute_symbol_location(
    s_i: i32,
    t_i: i32,
    w_i: i32,
    h_i: i32,
    transposed: bool,
    refcorner: ReferenceCorner,
) -> (i32, i32) {
    if !transposed {
        // "If TRANSPOSED is 0, then:"
        match refcorner {
            // "If REFCORNER is TOPLEFT then the top left pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[S_I, T_I]."
            ReferenceCorner::TopLeft => (s_i, t_i),
            // "If REFCORNER is TOPRIGHT then the top right pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[S_I, T_I]."
            ReferenceCorner::TopRight => (s_i - w_i + 1, t_i),
            // "If REFCORNER is BOTTOMLEFT then the bottom left pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[S_I, T_I]."
            ReferenceCorner::BottomLeft => (s_i, t_i - h_i + 1),
            // "If REFCORNER is BOTTOMRIGHT then the bottom right pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[S_I, T_I]."
            ReferenceCorner::BottomRight => (s_i - w_i + 1, t_i - h_i + 1),
        }
    } else {
        // "If TRANSPOSED is 1, then:"
        match refcorner {
            // "If REFCORNER is TOPLEFT then the top left pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[T_I, S_I]."
            ReferenceCorner::TopLeft => (t_i, s_i),
            // "If REFCORNER is TOPRIGHT then the top right pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[T_I, S_I]."
            ReferenceCorner::TopRight => (t_i - w_i + 1, s_i),
            // "If REFCORNER is BOTTOMLEFT then the bottom left pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[T_I, S_I]."
            ReferenceCorner::BottomLeft => (t_i, s_i - h_i + 1),
            // "If REFCORNER is BOTTOMRIGHT then the bottom right pixel of the symbol
            // instance bitmap IB_I shall be placed at SBREG[T_I, S_I]."
            ReferenceCorner::BottomRight => (t_i - w_i + 1, s_i - h_i + 1),
        }
    }
}

/// Draw a symbol bitmap into the region using the specified combination operator.
fn draw_symbol(
    sbreg: &mut DecodedRegion,
    symbol: &DecodedRegion,
    x: i32,
    y: i32,
    combop: CombinationOperator,
) {
    for sy in 0..symbol.height {
        let dest_y = y + sy as i32;
        if dest_y < 0 || dest_y >= sbreg.height as i32 {
            continue;
        }

        for sx in 0..symbol.width {
            let dest_x = x + sx as i32;
            if dest_x < 0 || dest_x >= sbreg.width as i32 {
                continue;
            }

            let src_pixel = symbol.get_pixel(sx, sy);
            let dst_pixel = sbreg.get_pixel(dest_x as u32, dest_y as u32);

            let result = match combop {
                CombinationOperator::Or => dst_pixel | src_pixel,
                CombinationOperator::And => dst_pixel & src_pixel,
                CombinationOperator::Xor => dst_pixel ^ src_pixel,
                CombinationOperator::Xnor => !(dst_pixel ^ src_pixel),
                CombinationOperator::Replace => src_pixel,
            };

            sbreg.set_pixel(dest_x as u32, dest_y as u32, result);
        }
    }
}

/// Select Huffman tables based on flags (7.4.3.1.6).
fn select_huffman_tables<'a>(
    flags: &TextRegionHuffmanFlags,
    custom_tables: &[&'a HuffmanTable],
) -> Result<TextRegionHuffmanTables<'a>, &'static str> {
    let mut custom_idx = 0;

    let mut get_custom = || -> Result<&'a HuffmanTable, &'static str> {
        let table = custom_tables[custom_idx];

        custom_idx += 1;
        Ok(table)
    };

    // "1) SBHUFFFS"
    let sbhufffs: &HuffmanTable = match flags.sbhufffs {
        0 => &TABLE_F,
        1 => &TABLE_G,
        3 => get_custom()?,
        _ => return Err("invalid SBHUFFFS selection"),
    };

    // "2) SBHUFFDS"
    let sbhuffds: &HuffmanTable = match flags.sbhuffds {
        0 => &TABLE_H,
        1 => &TABLE_I,
        2 => &TABLE_J,
        3 => get_custom()?,
        _ => return Err("invalid SBHUFFDS selection"),
    };

    // "3) SBHUFFDT"
    let sbhuffdt: &HuffmanTable = match flags.sbhuffdt {
        0 => &TABLE_K,
        1 => &TABLE_L,
        2 => &TABLE_M,
        3 => get_custom()?,
        _ => return Err("invalid SBHUFFDT selection"),
    };

    // "4) SBHUFFRDW"
    let sbhuffrdw: &HuffmanTable = match flags.sbhuffrdw {
        0 => &TABLE_N,
        1 => &TABLE_O,
        3 => get_custom()?,
        _ => return Err("invalid SBHUFFRDW selection"),
    };

    // "5) SBHUFFRDH"
    let sbhuffrdh: &HuffmanTable = match flags.sbhuffrdh {
        0 => &TABLE_N,
        1 => &TABLE_O,
        3 => get_custom()?,
        _ => return Err("invalid SBHUFFRDH selection"),
    };

    // "6) SBHUFFRDY"
    let sbhuffrdy: &HuffmanTable = match flags.sbhuffrdy {
        0 => &TABLE_N,
        1 => &TABLE_O,
        3 => get_custom()?,
        _ => return Err("invalid SBHUFFRDY selection"),
    };

    // "7) SBHUFFRDX"
    let sbhuffrdx: &HuffmanTable = match flags.sbhuffrdx {
        0 => &TABLE_N,
        1 => &TABLE_O,
        3 => get_custom()?,
        _ => return Err("invalid SBHUFFRDX selection"),
    };

    // "8) SBHUFFRSIZE"
    let sbhuffrsize: &HuffmanTable = match flags.sbhuffrsize {
        0 => &TABLE_A,
        1 => get_custom()?,
        _ => return Err("invalid SBHUFFRSIZE selection"),
    };

    Ok(TextRegionHuffmanTables {
        sbhufffs,
        sbhuffds,
        sbhuffdt,
        sbhuffrdw,
        sbhuffrdh,
        sbhuffrdy,
        sbhuffrdx,
        sbhuffrsize,
    })
}

/// Decode a text region using Huffman coding (SBHUFF=1).
fn decode_text_region_huffman(
    reader: &mut Reader<'_>,
    symbols: &[&DecodedRegion],
    header: &TextRegionHeader,
    params: &TextRegionParams<'_>,
    referred_tables: &[&HuffmanTable],
) -> Result<DecodedRegion, &'static str> {
    let huffman_flags = header
        .huffman_flags
        .as_ref()
        .ok_or("missing huffman flags for SBHUFF=1")?;

    let custom_count = [
        huffman_flags.sbhufffs == 3,
        huffman_flags.sbhuffds == 3,
        huffman_flags.sbhuffdt == 3,
        huffman_flags.sbhuffrdw == 3,
        huffman_flags.sbhuffrdh == 3,
        huffman_flags.sbhuffrdy == 3,
        huffman_flags.sbhuffrdx == 3,
        huffman_flags.sbhuffrsize == 1,
    ]
    .into_iter()
    .filter(|x| *x)
    .count();

    if referred_tables.len() < custom_count {
        return Err("not enough referred huffman tables");
    }

    let tables = select_huffman_tables(huffman_flags, referred_tables)?;

    let sbnumsyms = symbols.len() as u32;
    let sbsymcodes = decode_symbol_id_huffman_table(reader, sbnumsyms)?;

    let sbw = params.sbw;
    let sbh = params.sbh;
    let sbnuminstances = params.sbnuminstances;
    let sbstrips = params.sbstrips;
    let sbdefpixel = params.sbdefpixel;
    let transposed = params.transposed;
    let refcorner = params.refcorner;
    let sbdsoffset = params.sbdsoffset;
    let sbcombop = params.sbcombop;
    let sbrefine = header.flags.sbrefine;
    let log_sbstrips = header.flags.log_sb_strips;

    // "1) Fill a bitmap SBREG, of the size given by SBW and SBH, with the
    // SBDEFPIXEL value." (6.4.5)
    let mut sbreg = DecodedRegion::new(sbw, sbh);
    if sbdefpixel {
        for pixel in &mut sbreg.data {
            *pixel = true;
        }
    }

    // "2) Decode the initial STRIPT value as described in 6.4.6." (6.4.5)
    // "If SBHUFF is 1, decode a value using the Huffman table specified by
    // SBHUFFDT and multiply the resulting value by SBSTRIPS." (6.4.6)
    let initial_stript = decode_huffman_value(tables.sbhuffdt, reader)? * sbstrips as i32;
    let mut stript: i32 = -initial_stript;
    let mut firsts: i32 = 0;
    let mut ninstances: u32 = 0;

    // "4) Decode each strip as follows:" (6.4.5)
    while ninstances < sbnuminstances {
        // "b) Decode the strip's delta T value as described in 6.4.6."
        let dt = decode_huffman_value(tables.sbhuffdt, reader)? * sbstrips as i32;
        stript += dt;

        // "c) Decode each symbol instance in the strip"
        let mut first_symbol_in_strip = true;
        let mut curs: i32 = 0;

        loop {
            if first_symbol_in_strip {
                // "i) First symbol instance's S coordinate (6.4.7)
                // If SBHUFF is 1, decode a value using the Huffman table
                // specified by SBHUFFFS." (6.4.7)
                let dfs = decode_huffman_value(tables.sbhufffs, reader)?;
                firsts += dfs;
                curs = firsts;
                first_symbol_in_strip = false;
            } else {
                // "ii) Subsequent symbol instance S coordinate (6.4.8)
                // If SBHUFF is 1, decode a value using the Huffman table
                // specified by SBHUFFDS." (6.4.8)
                let Some(ids) = tables.sbhuffds.decode(reader)? else {
                    // End of strip (OOB).
                    break;
                };

                curs = curs + ids + sbdsoffset;
            }

            // "iii) Symbol instance T coordinate (6.4.9)
            // If SBSTRIPS = 1, then the value decoded is always zero.
            // If SBHUFF is 1, decode a value by reading ceil(log2(SBSTRIPS))
            // bits directly from the bitstream." (6.4.9)
            let curt = if sbstrips == 1 {
                0
            } else {
                reader.read_bits(log_sbstrips)? as i32
            };
            let t_i = stript + curt;

            // "iv) Symbol instance symbol ID (6.4.10)
            // If SBHUFF is 1, decode a value by reading one bit at a time until
            // the resulting bit string is equal to one of the entries in
            // SBSYMCODES." (6.4.10)
            let id_i = decode_huffman_value(&sbsymcodes, reader)? as usize;

            // "v) Determine the symbol instance's bitmap IB_I as described in
            // 6.4.11." (6.4.5)
            let (ib_i, w_i, h_i): (std::borrow::Cow<'_, DecodedRegion>, i32, i32) = if !sbrefine {
                // "If SBREFINE is 0, then set R_I to 0." (6.4.11)
                let sym = symbols.get(id_i).ok_or("symbol ID out of range")?;
                (
                    std::borrow::Cow::Borrowed(*sym),
                    sym.width as i32,
                    sym.height as i32,
                )
            } else {
                // "If SBREFINE is 1, then decode R_I as follows:
                // If SBHUFF is 1, then read one bit and set R_I to the value
                // of that bit." (6.4.11)
                let r_i = reader.read_bit().ok_or("unexpected end reading R_I")?;

                if r_i == 0 {
                    let sym = symbols.get(id_i).ok_or("symbol ID out of range")?;
                    (
                        std::borrow::Cow::Borrowed(*sym),
                        sym.width as i32,
                        sym.height as i32,
                    )
                } else {
                    // Refinement decoding (6.4.11)
                    let ibo_i = symbols.get(id_i).ok_or("symbol ID out of range")?;
                    let wo_i = ibo_i.width;
                    let ho_i = ibo_i.height;

                    // "1) Decode the symbol instance refinement delta width"
                    let rdw_i = decode_huffman_value(tables.sbhuffrdw, reader)?;

                    // "2) Decode the symbol instance refinement delta height"
                    let rdh_i = decode_huffman_value(tables.sbhuffrdh, reader)?;

                    // "3) Decode the symbol instance refinement X offset"
                    let rdx_i = decode_huffman_value(tables.sbhuffrdx, reader)?;

                    // "4) Decode the symbol instance refinement Y offset"
                    let rdy_i = decode_huffman_value(tables.sbhuffrdy, reader)?;

                    // "5) If SBHUFF is 1, then:
                    // a) Decode the symbol instance refinement bitmap data size
                    // b) Skip over any bits remaining in the last byte read"
                    let rsize = decode_huffman_value(tables.sbhuffrsize, reader)? as u32;
                    reader.align();

                    // "6) Decode the refinement bitmap"
                    let grw = (wo_i as i32 + rdw_i) as u32;
                    let grh = (ho_i as i32 + rdh_i) as u32;
                    let grreferencedx = rdw_i.div_euclid(2) + rdx_i;
                    let grreferencedy = rdh_i.div_euclid(2) + rdy_i;

                    let mut refined = DecodedRegion::new(grw, grh);

                    // Read the refinement data (rsize bytes)
                    let refinement_data = reader
                        .read_bytes(rsize as usize)
                        .ok_or("unexpected end reading refinement data")?;

                    // Decode refinement bitmap from raw bytes.
                    // TPGRON is always 0 for text region refinements (Table 12).
                    let mut decoder = ArithmeticDecoder::new(refinement_data);
                    let num_context_bits = match params.sbrtemplate {
                        GrTemplate::Template0 => 13,
                        GrTemplate::Template1 => 10,
                    };
                    let mut contexts =
                        vec![ArithmeticDecoderContext::default(); 1 << num_context_bits];

                    decode_refinement_bitmap_with(
                        &mut decoder,
                        &mut contexts,
                        &mut refined,
                        ibo_i,
                        grreferencedx,
                        grreferencedy,
                        params.sbrtemplate,
                        params.refinement_at_pixels,
                        false, // TPGRON = 0
                    )?;

                    (std::borrow::Cow::Owned(refined), grw as i32, grh as i32)
                }
            };

            // "vi) Update CURS as follows:"
            if !transposed
                && (refcorner == ReferenceCorner::TopRight
                    || refcorner == ReferenceCorner::BottomRight)
            {
                curs += w_i - 1;
            } else if transposed
                && (refcorner == ReferenceCorner::BottomLeft
                    || refcorner == ReferenceCorner::BottomRight)
            {
                curs += h_i - 1;
            }

            // "vii) Set: S_I = CURS"
            let s_i = curs;

            // "viii) Determine the location"
            let (x, y) = compute_symbol_location(s_i, t_i, w_i, h_i, transposed, refcorner);

            // "x) Draw IB_I into SBREG"
            draw_symbol(&mut sbreg, &ib_i, x, y, sbcombop);

            // "xi) Update CURS"
            if !transposed
                && (refcorner == ReferenceCorner::TopLeft
                    || refcorner == ReferenceCorner::BottomLeft)
            {
                curs += w_i - 1;
            } else if transposed
                && (refcorner == ReferenceCorner::TopLeft || refcorner == ReferenceCorner::TopRight)
            {
                curs += h_i - 1;
            }

            // "xii) Set: NINSTANCES = NINSTANCES + 1"
            ninstances += 1;

            if ninstances >= sbnuminstances {
                break;
            }
        }
    }

    Ok(sbreg)
}

/// Decode the symbol ID Huffman table (7.4.3.1.7).
///
/// "This table is encoded as SBNUMSYMS symbol ID code lengths; the actual codes
/// in SBSYMCODES are assigned from these symbol ID code lengths using the
/// algorithm in B.3.
///
/// The symbol ID code lengths themselves are run-length coded and the runs
/// Huffman coded. This is very similar to the 'zlib' coded format documented
/// in RFC 1951, though not identical. The encoding is based on the codes shown
/// in Table 29." (7.4.3.1.7)
fn decode_symbol_id_huffman_table(
    reader: &mut Reader<'_>,
    sbnumsyms: u32,
) -> Result<HuffmanTable, &'static str> {
    // "1) Read the code lengths for RUNCODE0 through RUNCODE34; each is stored
    // as a four-bit value." (7.4.3.1.7)
    let mut runcode_lines: Vec<TableLine> = Vec::with_capacity(35);
    for i in 0..35 {
        let preflen = reader.read_bits(4)? as u8;
        runcode_lines.push(TableLine::new(i, preflen, 0));
    }

    // "2) Given the lengths, assign Huffman codes for RUNCODE0 through RUNCODE34
    // using the algorithm in B.3." (7.4.3.1.7)
    let runcode_table = HuffmanTable::build(&runcode_lines);

    // "3) Read a Huffman code using this assignment. This decodes into one of
    // RUNCODE0 through RUNCODE34." (7.4.3.1.7)
    // "5) Repeat steps 3) and 4) until the symbol ID code lengths for all
    // SBNUMSYMS symbols have been determined." (7.4.3.1.7)
    let mut symbol_code_lengths = Vec::with_capacity(sbnumsyms as usize);

    while symbol_code_lengths.len() < sbnumsyms as usize {
        let runcode = decode_huffman_value(&runcode_table, reader)? as u32;

        // "4) Interpret the RUNCODE code and the additional bits (if any)
        // according to Table 29. This gives the symbol ID code lengths for
        // one or more symbols." (7.4.3.1.7)
        //
        // Table 32 – Meaning of the run codes:
        // RUNCODE0-31: Symbol ID code length is 0-31
        // RUNCODE32: Copy previous length 3-6 times (2 extra bits + 3)
        // RUNCODE33: Repeat 0 length 3-10 times (3 extra bits + 3)
        // RUNCODE34: Repeat 0 length 11-138 times (7 extra bits + 11)
        match runcode {
            0..=31 => {
                symbol_code_lengths.push(runcode as u8);
            }
            32 => {
                // Copy previous 3-6 times
                let extra = reader.read_bits(2)? as usize;
                let repeat = extra + 3;
                let prev = *symbol_code_lengths
                    .last()
                    .ok_or("RUNCODE32 with no previous length")?;
                for _ in 0..repeat {
                    if symbol_code_lengths.len() >= sbnumsyms as usize {
                        break;
                    }
                    symbol_code_lengths.push(prev);
                }
            }
            33 => {
                // Repeat 0 length 3-10 times
                let extra = reader.read_bits(3)? as usize;
                let repeat = extra + 3;
                for _ in 0..repeat {
                    if symbol_code_lengths.len() >= sbnumsyms as usize {
                        break;
                    }
                    symbol_code_lengths.push(0);
                }
            }
            34 => {
                // Repeat 0 length 11-138 times
                let extra = reader.read_bits(7)? as usize;
                let repeat = extra + 11;
                for _ in 0..repeat {
                    if symbol_code_lengths.len() >= sbnumsyms as usize {
                        break;
                    }
                    symbol_code_lengths.push(0);
                }
            }
            _ => return Err("invalid runcode"),
        }
    }

    // "6) Skip over the remaining bits in the last byte read, so that the actual
    // text region decoding procedure begins on a byte boundary." (7.4.3.1.7)
    reader.align();

    // "7) Assign a Huffman code to each symbol by applying the algorithm in B.3
    // to the symbol ID code lengths just decoded. The result is the symbol ID
    // Huffman table SBSYMCODES." (7.4.3.1.7)
    let symbol_lines: Vec<TableLine> = symbol_code_lengths
        .iter()
        .enumerate()
        .map(|(idx, &preflen)| TableLine::new(idx as i32, preflen, 0))
        .collect();
    Ok(HuffmanTable::build(&symbol_lines))
}

/// Collection of Huffman tables for text region decoding.
struct TextRegionHuffmanTables<'a> {
    sbhufffs: &'a HuffmanTable,
    sbhuffds: &'a HuffmanTable,
    sbhuffdt: &'a HuffmanTable,
    sbhuffrdw: &'a HuffmanTable,
    sbhuffrdh: &'a HuffmanTable,
    sbhuffrdy: &'a HuffmanTable,
    sbhuffrdx: &'a HuffmanTable,
    sbhuffrsize: &'a HuffmanTable,
}

/// Decode a value from a Huffman table, requiring a value (not OOB).
fn decode_huffman_value(
    table: &HuffmanTable,
    reader: &mut Reader<'_>,
) -> Result<i32, &'static str> {
    table
        .decode(reader)?
        .ok_or("unexpected OOB in huffman decode")
}
