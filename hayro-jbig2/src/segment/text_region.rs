//! Text region segment parsing and decoding (7.4.3, 6.4).
//!
//! "The data parts of all three of the text region segment types ('intermediate
//! text region', 'immediate text region' and 'immediate lossless text region')
//! are coded identically, but are acted upon differently, see 8.2. The syntax
//! of these segment types' data parts is specified here." (7.4.3)

use crate::arithmetic_decoder::{
    ArithmeticDecoder, ArithmeticDecoderContext, IntegerDecoder, SymbolIdDecoder,
};
use crate::bitmap::DecodedRegion;
use crate::reader::Reader;
use crate::segment::generic_refinement_region::{
    GrTemplate, RefinementAdaptiveTemplatePixel, decode_refinement_bitmap_with,
};
use crate::segment::region::{CombinationOperator, RegionSegmentInfo, parse_region_segment_info};

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

/// Parse a text region segment header (7.4.3.1).
pub(crate) fn parse_text_region_header(
    reader: &mut Reader<'_>,
) -> Result<TextRegionHeader, &'static str> {
    // "Region segment information field – see 7.4.1."
    let region_info = parse_region_segment_info(reader)?;

    // "Text region segment flags – see 7.4.3.1.1."
    let flags = parse_text_region_flags(reader)?;

    // Check for unsupported Huffman coding early
    if flags.sbhuff {
        return Err("SBHUFF=1 (Huffman coding) is not supported for text regions");
    }

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
    pub fn from_header(header: &'a TextRegionHeader) -> Self {
        let sbrtemplate = if header.flags.sbrtemplate == 0 {
            GrTemplate::Template0
        } else {
            GrTemplate::Template1
        };

        Self {
            sbw: header.region_info.width,
            sbh: header.region_info.height,
            sbnuminstances: header.num_instances,
            sbstrips: 1u32 << header.flags.log_sb_strips,
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
pub(crate) fn decode_text_region(
    reader: &mut Reader<'_>,
    symbols: &[&DecodedRegion],
) -> Result<DecodedRegion, &'static str> {
    let header = parse_text_region_header(reader)?;
    let data = reader.tail().ok_or("unexpected end of data")?;
    let mut decoder = ArithmeticDecoder::new(data);
    let params = TextRegionParams::from_header(&header);

    let mut sbreg = if header.flags.sbrefine {
        decode_text_region_refine(&mut decoder, symbols, &params)?
    } else {
        decode_text_region_direct(&mut decoder, symbols, &params)?
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
    decode_text_region_with(decoder, symbols, params, |_decoder, id_i, _symbols| {
        // "If SBREFINE is 0, then set R_I to 0." (6.4.11)
        // "If R_I is 0 then set the symbol instance bitmap IB_I to SBSYMS[ID_I]."
        Ok(SymbolBitmap::Reference(id_i))
    })
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
    // Additional decoders for refinement (6.4.11)
    let mut iari = IntegerDecoder::new(); // R_I decoder
    let mut iardw = IntegerDecoder::new(); // RDW_I decoder
    let mut iardh = IntegerDecoder::new(); // RDH_I decoder
    let mut iardx = IntegerDecoder::new(); // RDX_I decoder
    let mut iardy = IntegerDecoder::new(); // RDY_I decoder

    // Refinement contexts
    let num_gr_contexts = match params.sbrtemplate {
        GrTemplate::Template0 => 1 << 13,
        GrTemplate::Template1 => 1 << 10,
    };
    let mut gr_contexts = vec![ArithmeticDecoderContext::default(); num_gr_contexts];

    decode_text_region_with(decoder, symbols, params, |decoder, id_i, syms| {
        // "If SBREFINE is 1, then decode R_I as follows:
        //  If SBHUFF is 0, decode one bit using the IARI integer arithmetic
        //  decoding procedure and set R_I to the value of that bit." (6.4.11)
        let r_i = iari.decode(decoder).ok_or("unexpected OOB decoding R_I")?;

        if r_i == 0 {
            // "If R_I is 0 then set the symbol instance bitmap IB_I to SBSYMS[ID_I]."
            Ok(SymbolBitmap::Reference(id_i))
        } else {
            // "If R_I is 1 then determine the symbol instance bitmap as follows:"
            // (6.4.11)
            let ibo_i = syms.get(id_i).ok_or("symbol ID out of range")?;
            let wo_i = ibo_i.width;
            let ho_i = ibo_i.height;

            // "1) Decode the symbol instance refinement delta width as described
            // in 6.4.11.1. Let RDW_I be the value decoded." (6.4.11)
            let rdw_i = iardw
                .decode(decoder)
                .ok_or("unexpected OOB decoding RDW_I")?;

            // "2) Decode the symbol instance refinement delta height as described
            // in 6.4.11.2. Let RDH_I be the value decoded." (6.4.11)
            let rdh_i = iardh
                .decode(decoder)
                .ok_or("unexpected OOB decoding RDH_I")?;

            // "3) Decode the symbol instance refinement X offset as described in
            // 6.4.11.3. Let RDX_I be the value decoded." (6.4.11)
            let rdx_i = iardx
                .decode(decoder)
                .ok_or("unexpected OOB decoding RDX_I")?;

            // "4) Decode the symbol instance refinement Y offset as described in
            // 6.4.11.4. Let RDY_I be the value decoded." (6.4.11)
            let rdy_i = iardy
                .decode(decoder)
                .ok_or("unexpected OOB decoding RDY_I")?;

            // "6) Let IBO_I be SBSYMS[ID_I]. Let WO_I be the width of IBO_I and
            // HO_I be the height of IBO_I. The symbol instance bitmap IB_I is the
            // result of applying the generic refinement region decoding procedure
            // described in 6.3. Set the parameters to this decoding procedure as
            // shown in Table 12." (6.4.11)
            //
            // Table 12 parameters:
            // GRW = WO_I + RDW_I
            // GRH = HO_I + RDH_I
            // GRTEMPLATE = SBRTEMPLATE
            // GRREFERENCE = IBO_I
            // GRREFERENCEDX = floor(RDW_I/2) + RDX_I
            // GRREFERENCEDY = floor(RDH_I/2) + RDY_I
            // TPGRON = 0
            // GRATXn = SBRATXn, GRATYn = SBRATYn

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
                false, // TPGRON = 0
            )?;

            Ok(SymbolBitmap::Owned(refined))
        }
    })
}

/// Result of determining a symbol instance bitmap.
enum SymbolBitmap {
    /// Use the symbol at this index directly (R_I = 0).
    Reference(usize),
    /// Use this refined bitmap (R_I = 1).
    Owned(DecodedRegion),
}

/// Core text region decoding loop (6.4.5).
///
/// Takes a closure that determines each symbol instance bitmap.
fn decode_text_region_with<F>(
    decoder: &mut ArithmeticDecoder<'_>,
    symbols: &[&DecodedRegion],
    params: &TextRegionParams<'_>,
    mut get_symbol_bitmap: F,
) -> Result<DecodedRegion, &'static str>
where
    F: FnMut(
        &mut ArithmeticDecoder<'_>,
        usize,
        &[&DecodedRegion],
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

    // Calculate SBSYMCODELEN = ceil(log2(SBNUMSYMS))
    // "SBSYMCODELEN: Integer, 6 N, The length of the symbol codes used in IAID.d)"
    let sbnumsyms = symbols.len() as u32;
    if sbnumsyms == 0 {
        return Err("text region has no symbols");
    }
    let sbsymcodelen = 32 - (sbnumsyms - 1).leading_zeros();

    // Initialize integer decoders
    let mut iadt = IntegerDecoder::new(); // Strip delta T
    let mut iafs = IntegerDecoder::new(); // First symbol S coordinate
    let mut iads = IntegerDecoder::new(); // Subsequent symbol S coordinate
    let mut iait = IntegerDecoder::new(); // Symbol T coordinate
    let mut iaid = SymbolIdDecoder::new(sbsymcodelen);

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
    let initial_stript = decode_strip_delta_t(decoder, &mut iadt, sbstrips)?;
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
        let dt = decode_strip_delta_t(decoder, &mut iadt, sbstrips)?;
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
                let dfs = iafs
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
                match iads.decode(decoder) {
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
            let curt = decode_symbol_t_coordinate(decoder, &mut iait, sbstrips)?;
            let t_i = stript + curt;

            // "iv) Decode the symbol instance's symbol ID as described in 6.4.10.
            // Let ID_I be the decoded value." (6.4.5)
            let id_i = iaid.decode(decoder) as usize;

            // "v) Determine the symbol instance's bitmap IB_I as described in 6.4.11.
            // The width and height of this bitmap shall be denoted as W_I and H_I
            // respectively." (6.4.5)
            let symbol_bitmap = get_symbol_bitmap(decoder, id_i, symbols)?;
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
