//! Pattern dictionary segment parsing and decoding (7.4.4, 6.7).

use crate::bitmap::DecodedRegion;
use crate::reader::Reader;
use crate::segment::generic_region::{
    AdaptiveTemplatePixel, GbTemplate, decode_bitmap_arith, decode_bitmap_mmr,
};
use crate::segment::region::CombinationOperator;

/// Template used for pattern dictionary arithmetic coding (7.4.4.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HdTemplate {
    /// Template 0
    Template0 = 0,
    /// Template 1
    Template1 = 1,
    /// Template 2
    Template2 = 2,
    /// Template 3
    Template3 = 3,
}

impl HdTemplate {
    fn from_value(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(Self::Template0),
            1 => Ok(Self::Template1),
            2 => Ok(Self::Template2),
            3 => Ok(Self::Template3),
            _ => Err("invalid pattern dictionary template"),
        }
    }

    /// Convert to GbTemplate for generic region decoding.
    fn to_gb_template(self) -> GbTemplate {
        match self {
            HdTemplate::Template0 => GbTemplate::Template0,
            HdTemplate::Template1 => GbTemplate::Template1,
            HdTemplate::Template2 => GbTemplate::Template2,
            HdTemplate::Template3 => GbTemplate::Template3,
        }
    }
}

/// Parsed pattern dictionary segment flags (7.4.4.1.1).
///
/// "This one-byte field is formatted as shown in Figure 42."
#[derive(Debug, Clone)]
pub(crate) struct PatternDictionaryFlags {
    /// "Bit 0: HDMMR. If this bit is 1, then the segment uses the MMR encoding
    /// variant. If this bit is 0, then the segment uses the arithmetic encoding
    /// variant."
    pub hdmmr: bool,
    /// "Bits 1-2: HDTEMPLATE. This field controls the template used to decode
    /// patterns if HDMMR is 0. If HDMMR is 1, this field must contain the
    /// value 0."
    pub hdtemplate: HdTemplate,
}

/// Parsed pattern dictionary segment header (7.4.4.1).
///
/// "A pattern dictionary segment's data part begins with a pattern dictionary
/// segment data header, formatted as shown in Figure 41."
#[derive(Debug, Clone)]
pub(crate) struct PatternDictionaryHeader {
    /// Pattern dictionary flags (7.4.4.1.1).
    pub flags: PatternDictionaryFlags,
    /// "HDPW: This one-byte field contains the width of the patterns defined
    /// in this pattern dictionary. Its value must be greater than zero."
    /// (7.4.4.1.2)
    pub hdpw: u8,
    /// "HDPH: This one-byte field contains the height of the patterns defined
    /// in this pattern dictionary. Its value must be greater than zero."
    /// (7.4.4.1.3)
    pub hdph: u8,
    /// "GRAYMAX: This four-byte field contains one less than the number of
    /// patterns defined in this pattern dictionary." (7.4.4.1.4)
    pub graymax: u32,
}

/// A decoded pattern dictionary containing GRAYMAX + 1 patterns.
///
/// "The patterns exported by this pattern dictionary. Contains GRAYMAX + 1
/// patterns." (Table 25)
#[derive(Debug, Clone)]
pub(crate) struct PatternDictionary {
    /// The patterns in this dictionary, indexed 0 through GRAYMAX.
    pub patterns: Vec<DecodedRegion>,
    /// Width of each pattern.
    pub pattern_width: u32,
    /// Height of each pattern.
    pub pattern_height: u32,
}

/// Parse a pattern dictionary segment header (7.4.4.1).
pub(crate) fn parse_pattern_dictionary_header(
    reader: &mut Reader<'_>,
) -> Result<PatternDictionaryHeader, &'static str> {
    // 7.4.4.1.1: Pattern dictionary flags
    let flags_byte = reader.read_byte().ok_or("unexpected end of data")?;

    // "Bit 0: HDMMR"
    let hdmmr = flags_byte & 0x01 != 0;

    // "Bits 1-2: HDTEMPLATE"
    let hdtemplate = HdTemplate::from_value((flags_byte >> 1) & 0x03)?;

    // "Bits 3-7: Reserved; must be 0."
    if flags_byte & 0xF8 != 0 {
        return Err("reserved bits in pattern dictionary flags must be 0");
    }

    // Validate constraint: HDTEMPLATE must be 0 when HDMMR is 1
    if hdmmr && hdtemplate != HdTemplate::Template0 {
        return Err("HDTEMPLATE must be 0 when HDMMR is 1");
    }

    let flags = PatternDictionaryFlags { hdmmr, hdtemplate };

    // 7.4.4.1.2: HDPW - Width of patterns
    let hdpw = reader.read_byte().ok_or("unexpected end of data")?;
    if hdpw == 0 {
        return Err("HDPW must be greater than zero");
    }

    // 7.4.4.1.3: HDPH - Height of patterns
    let hdph = reader.read_byte().ok_or("unexpected end of data")?;
    if hdph == 0 {
        return Err("HDPH must be greater than zero");
    }

    // 7.4.4.1.4: GRAYMAX - One less than number of patterns
    let graymax = reader.read_u32().ok_or("unexpected end of data")?;

    Ok(PatternDictionaryHeader {
        flags,
        hdpw,
        hdph,
        graymax,
    })
}

/// Decode a pattern dictionary segment (7.4.4.2, 6.7).
///
/// "A pattern dictionary segment is decoded according to the following steps:
/// 1) Interpret its header, as described in 7.4.4.1.
/// 2) As described in E.3.7, reset all the arithmetic coding statistics to zero.
/// 3) Invoke the pattern dictionary decoding procedure described in 6.7."
pub(crate) fn decode_pattern_dictionary(
    reader: &mut Reader<'_>,
) -> Result<PatternDictionary, &'static str> {
    let header = parse_pattern_dictionary_header(reader)?;

    let hdpw = header.hdpw as u32;
    let hdph = header.hdph as u32;
    let num_patterns = header.graymax.checked_add(1).ok_or("GRAYMAX overflow")?;

    // "1) Create a bitmap B_HDC. The height of this bitmap is HDPH. The width
    // of the bitmap is (GRAYMAX + 1) × HDPW. This bitmap contains all the
    // patterns concatenated left to right." (6.7.5)
    let collective_width = num_patterns
        .checked_mul(hdpw)
        .ok_or("collective bitmap width overflow")?;

    // Get the remaining data for decoding.
    let encoded_data = reader.tail().ok_or("unexpected end of data")?;

    // Create the collective bitmap.
    let mut collective_bitmap = DecodedRegion {
        width: collective_width,
        height: hdph,
        data: vec![false; (collective_width * hdph) as usize],
        x_location: 0,
        y_location: 0,
        combination_operator: CombinationOperator::Replace,
    };

    // "2) Decode the collective bitmap using a generic region decoding procedure
    // as described in 6.2. Set the parameters to this decoding procedure as
    // shown in Table 27." (6.7.5)
    if header.flags.hdmmr {
        let _ = decode_bitmap_mmr(&mut collective_bitmap, encoded_data)?;
    } else {
        // Build AT pixels according to Table 27.
        let at_pixels = build_pattern_at_pixels(header.flags.hdtemplate, hdpw);
        decode_bitmap_arith(
            &mut collective_bitmap,
            encoded_data,
            header.flags.hdtemplate.to_gb_template(),
            false, // TPGDON = 0 (Table 27)
            &at_pixels,
        )?;
    }

    // "3) Set: GRAY = 0" (6.7.5)
    // "4) While GRAY ≤ GRAYMAX:" (6.7.5)
    let mut patterns = Vec::with_capacity(num_patterns as usize);

    for gray in 0..num_patterns {
        // "a) Let the subimage of B_HDC consisting of HPH rows and columns
        // HDPW × GRAY through HDPW × (GRAY + 1) − 1 be denoted B_P. Set:
        // HDPATS[GRAY] = B_P" (6.7.5)
        let start_x = gray * hdpw;
        let pattern = extract_pattern(&collective_bitmap, start_x, hdpw, hdph);
        patterns.push(pattern);
    }

    Ok(PatternDictionary {
        patterns,
        pattern_width: hdpw,
        pattern_height: hdph,
    })
}

/// Build adaptive template pixels for pattern dictionary decoding (Table 27).
fn build_pattern_at_pixels(hdtemplate: HdTemplate, hdpw: u32) -> Vec<AdaptiveTemplatePixel> {
    match hdtemplate {
        HdTemplate::Template0 => {
            vec![
                AdaptiveTemplatePixel {
                    x: -(hdpw as i8),
                    y: 0,
                },
                AdaptiveTemplatePixel { x: -3, y: -1 },
                AdaptiveTemplatePixel { x: 2, y: -2 },
                AdaptiveTemplatePixel { x: -2, y: -2 },
            ]
        }
        HdTemplate::Template1 | HdTemplate::Template2 | HdTemplate::Template3 => {
            vec![AdaptiveTemplatePixel {
                x: -(hdpw as i8),
                y: 0,
            }]
        }
    }
}

/// Extract a pattern from the collective bitmap.
///
/// "Let the subimage of B_HDC consisting of HPH rows and columns HDPW × GRAY
/// through HDPW × (GRAY + 1) − 1 be denoted B_P." (6.7.5)
fn extract_pattern(
    collective: &DecodedRegion,
    start_x: u32,
    width: u32,
    height: u32,
) -> DecodedRegion {
    let mut pattern = DecodedRegion::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let pixel = collective.get_pixel(start_x + x, y);
            pattern.set_pixel(x, y, pixel);
        }
    }

    pattern
}
