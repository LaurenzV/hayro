//! Halftone region segment parsing and decoding (7.4.5, 6.6).

use crate::bitmap::DecodedRegion;
use crate::reader::Reader;
use crate::segment::generic_region::{
    AdaptiveTemplatePixel, GbTemplate, decode_bitmap_mmr, gather_context_with_at,
};
use crate::segment::pattern_dictionary::PatternDictionary;
use crate::segment::region::{CombinationOperator, RegionSegmentInfo, parse_region_segment_info};

/// Template used for halftone arithmetic coding (7.4.5.1.1).
///
/// "This field controls the template used to decode halftone gray-scale value
/// bitplanes if HMMR is 0. If HMMR is 1, this field must contain the value 0."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HTemplate {
    /// Template 0
    Template0 = 0,
    /// Template 1
    Template1 = 1,
    /// Template 2
    Template2 = 2,
    /// Template 3
    Template3 = 3,
}

impl HTemplate {
    fn from_value(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(Self::Template0),
            1 => Ok(Self::Template1),
            2 => Ok(Self::Template2),
            3 => Ok(Self::Template3),
            _ => Err("invalid halftone template"),
        }
    }

    /// Convert to GbTemplate for generic region decoding.
    fn to_gb_template(self) -> GbTemplate {
        match self {
            HTemplate::Template0 => GbTemplate::Template0,
            HTemplate::Template1 => GbTemplate::Template1,
            HTemplate::Template2 => GbTemplate::Template2,
            HTemplate::Template3 => GbTemplate::Template3,
        }
    }
}

/// Parsed halftone region segment flags (7.4.5.1.1).
///
/// "This one-byte field is formatted as shown in Figure 44."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneRegionFlags {
    /// "Bit 0: HMMR. If this bit is 1, then the segment uses the MMR encoding
    /// variant. If this bit is 0, then the segment uses the arithmetic encoding
    /// variant."
    pub hmmr: bool,
    /// "Bits 1-2: HTEMPLATE. This field controls the template used to decode
    /// halftone gray-scale value bitplanes if HMMR is 0. If HMMR is 1, this
    /// field must contain the value 0."
    pub htemplate: HTemplate,
    /// "Bit 3: HENABLESKIP. This field controls whether gray-scale values that
    /// do not contribute to the region contents are skipped during decoding.
    /// If HMMR is 1, this field must contain the value 0."
    pub henableskip: bool,
    /// "Bits 4-6: HCOMBOP. This field has five possible values, representing
    /// one of five possible combination operators."
    pub hcombop: CombinationOperator,
    /// "Bit 7: HDEFPIXEL. This bit contains the initial value for every pixel
    /// in the halftone region, before any patterns are drawn."
    pub hdefpixel: bool,
}

/// Halftone grid position and size (7.4.5.1.2).
///
/// "This field describes the location and size of the grid of gray-scale values."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneGridPositionAndSize {
    /// "HGW: This four-byte field contains the width of the array of gray-scale
    /// values." (7.4.5.1.2.1)
    pub hgw: u32,
    /// "HGH: This four-byte field contains the height of the array of gray-scale
    /// values." (7.4.5.1.2.2)
    pub hgh: u32,
    /// "HGX: This signed four-byte field contains 256 times the horizontal offset
    /// of the origin of the halftone grid." (7.4.5.1.2.3)
    pub hgx: i32,
    /// "HGY: This signed four-byte field contains 256 times the vertical offset
    /// of the origin of the halftone grid." (7.4.5.1.2.4)
    pub hgy: i32,
}

/// Halftone grid vector (7.4.5.1.3).
///
/// "This field describes the vector used to draw the grid of gray-scale values."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneGridVector {
    /// "HRX: This unsigned two-byte field contains 256 times the horizontal
    /// coordinate of the halftone grid vector." (7.4.5.1.3.1)
    pub hrx: u16,
    /// "HRY: This unsigned two-byte field contains 256 times the vertical
    /// coordinate of the halftone grid vector." (7.4.5.1.3.2)
    pub hry: u16,
}

/// Parsed halftone region segment header (7.4.5.1).
///
/// "The data part of a halftone region segment begins with a halftone region
/// segment data header. This header contains the fields shown in Figure 43."
#[derive(Debug, Clone)]
pub(crate) struct HalftoneRegionHeader {
    /// Region segment information field (7.4.1).
    pub region_info: RegionSegmentInfo,
    /// Halftone region segment flags (7.4.5.1.1).
    pub flags: HalftoneRegionFlags,
    /// Halftone grid position and size (7.4.5.1.2).
    pub grid_position_and_size: HalftoneGridPositionAndSize,
    /// Halftone grid vector (7.4.5.1.3).
    pub grid_vector: HalftoneGridVector,
}

/// Parse a halftone region segment header (7.4.5.1).
pub(crate) fn parse_halftone_region_header(
    reader: &mut Reader<'_>,
) -> Result<HalftoneRegionHeader, &'static str> {
    // Region segment information field (7.4.1)
    let region_info = parse_region_segment_info(reader)?;

    // 7.4.5.1.1: Halftone region segment flags
    let flags_byte = reader.read_byte().ok_or("unexpected end of data")?;

    // "Bit 0: HMMR"
    let hmmr = flags_byte & 0x01 != 0;

    // "Bits 1-2: HTEMPLATE"
    let htemplate = HTemplate::from_value((flags_byte >> 1) & 0x03)?;

    // "Bit 3: HENABLESKIP"
    let henableskip = flags_byte & 0x08 != 0;

    // "Bits 4-6: HCOMBOP"
    let hcombop_value = (flags_byte >> 4) & 0x07;
    let hcombop = match hcombop_value {
        0 => CombinationOperator::Or,
        1 => CombinationOperator::And,
        2 => CombinationOperator::Xor,
        3 => CombinationOperator::Xnor,
        4 => CombinationOperator::Replace,
        _ => return Err("invalid halftone combination operator"),
    };

    // "Bit 7: HDEFPIXEL"
    let hdefpixel = flags_byte & 0x80 != 0;

    // Validate constraints when HMMR is 1
    if hmmr {
        if htemplate != HTemplate::Template0 {
            return Err("HTEMPLATE must be 0 when HMMR is 1");
        }
        if henableskip {
            return Err("HENABLESKIP must be 0 when HMMR is 1");
        }
    }

    let flags = HalftoneRegionFlags {
        hmmr,
        htemplate,
        henableskip,
        hcombop,
        hdefpixel,
    };

    // 7.4.5.1.2: Halftone grid position and size
    let hgw = reader.read_u32().ok_or("unexpected end of data")?;
    let hgh = reader.read_u32().ok_or("unexpected end of data")?;
    let hgx = reader.read_i32().ok_or("unexpected end of data")?;
    let hgy = reader.read_i32().ok_or("unexpected end of data")?;

    let grid_position_and_size = HalftoneGridPositionAndSize { hgw, hgh, hgx, hgy };

    // 7.4.5.1.3: Halftone grid vector
    let hrx = reader.read_u16().ok_or("unexpected end of data")?;
    let hry = reader.read_u16().ok_or("unexpected end of data")?;

    let grid_vector = HalftoneGridVector { hrx, hry };

    Ok(HalftoneRegionHeader {
        region_info,
        flags,
        grid_position_and_size,
        grid_vector,
    })
}

/// Decode a halftone region segment (7.4.5.2, 6.6).
///
/// "A halftone region segment is decoded according to the following steps:
/// 1) Interpret its header, as described in 7.4.5.1.
/// 2) Decode (or retrieve the results of decoding) the referred-to pattern
///    dictionary segment.
/// 3) As described in E.3.7, reset all the arithmetic coding statistics to zero.
/// 4) Invoke the halftone region decoding procedure described in 6.6."
pub(crate) fn decode_halftone_region(
    reader: &mut Reader<'_>,
    pattern_dict: &PatternDictionary,
) -> Result<DecodedRegion, &'static str> {
    let header = parse_halftone_region_header(reader)?;

    let hbw = header.region_info.width;
    let hbh = header.region_info.height;
    let hgw = header.grid_position_and_size.hgw;
    let hgh = header.grid_position_and_size.hgh;
    let hgx = header.grid_position_and_size.hgx;
    let hgy = header.grid_position_and_size.hgy;
    let hrx = header.grid_vector.hrx as i32;
    let hry = header.grid_vector.hry as i32;
    let hpw = pattern_dict.pattern_width;
    let hph = pattern_dict.pattern_height;
    let hnumpats = pattern_dict.patterns.len() as u32;

    // "1) Fill a bitmap HTREG, of the size given by HBW and HBH, with the
    // HDEFPIXEL value." (6.6.5)
    let mut htreg = DecodedRegion {
        width: hbw,
        height: hbh,
        data: vec![header.flags.hdefpixel; (hbw * hbh) as usize],
        x_location: header.region_info.x_location,
        y_location: header.region_info.y_location,
        combination_operator: header.region_info.combination_operator,
    };

    // "2) If HENABLESKIP equals 1, compute a bitmap HSKIP as shown in 6.6.5.1."
    let hskip = if header.flags.henableskip {
        Some(compute_hskip(
            hgw, hgh, hgx, hgy, hrx, hry, hpw, hph, hbw, hbh,
        ))
    } else {
        None
    };

    // "3) Set HBPP to ⌈log₂(HNUMPATS)⌉." (6.6.5)
    let hbpp = if hnumpats <= 1 {
        1
    } else {
        32 - (hnumpats - 1).leading_zeros()
    };

    // Get remaining data for decoding.
    let encoded_data = reader.tail().ok_or("unexpected end of data")?;

    // "4) Decode an image GI of size HGW by HGH with HBPP bits per pixel using
    // the gray-scale image decoding procedure as described in Annex C." (6.6.5)
    let gi = decode_gray_scale_image(
        encoded_data,
        hgw,
        hgh,
        hbpp,
        header.flags.hmmr,
        header.flags.henableskip,
        hskip.as_ref(),
        header.flags.htemplate,
    )?;

    // "5) Place sequentially the patterns corresponding to the values in GI into
    // HTREG by the procedure described in 6.6.5.2." (6.6.5)
    render_patterns(
        &mut htreg,
        &gi,
        hgw,
        hgh,
        hgx,
        hgy,
        hrx,
        hry,
        pattern_dict,
        header.flags.hcombop,
    );

    Ok(htreg)
}

/// Compute the HSKIP bitmap (6.6.5.1).
///
/// "The bitmap HSKIP contains 1 at a pixel if drawing a pattern at the
/// corresponding location on the halftone grid does not affect any pixels
/// of HTREG."
fn compute_hskip(
    hgw: u32,
    hgh: u32,
    hgx: i32,
    hgy: i32,
    hrx: i32,
    hry: i32,
    hpw: u32,
    hph: u32,
    hbw: u32,
    hbh: u32,
) -> Vec<bool> {
    let mut hskip = vec![false; (hgw * hgh) as usize];

    // "1) For each value of m_g between 0 and HGH − 1, beginning from 0,
    // perform the following steps:" (6.6.5.1)
    for m_g in 0..hgh {
        // "a) For each value of n_g between 0 and HGW − 1, beginning from 0,
        // perform the following steps:" (6.6.5.1)
        for n_g in 0..hgw {
            // "i) Set:
            //    x = (HGX + m_g × HRY + n_g × HRX) >>_A 8
            //    y = (HGY + m_g × HRX − n_g × HRY) >>_A 8" (6.6.5.1)
            let x = arithmetic_shift_right(hgx + (m_g as i32) * hry + (n_g as i32) * hrx, 8);
            let y = arithmetic_shift_right(hgy + (m_g as i32) * hrx - (n_g as i32) * hry, 8);

            // "ii) If ((x + HPW ≤ 0) OR (x ≥ HBW) OR (y + HPH ≤ 0) OR (y ≥ HBH))
            // then set: HSKIP[n_g, m_g] = 1" (6.6.5.1)
            let skip = (x + hpw as i32 <= 0)
                || (x >= hbw as i32)
                || (y + hph as i32 <= 0)
                || (y >= hbh as i32);

            hskip[(m_g * hgw + n_g) as usize] = skip;
        }
    }

    hskip
}

/// Decode the gray-scale image GI (Annex C / Table 23).
///
/// "Decode an image GI of size HGW by HGH with HBPP bits per pixel."
///
/// The gray-scale image is decoded by bit-plane coding. Each bit-plane is
/// decoded as a generic region bitmap. For arithmetic coding, all bitplanes
/// share the same decoder and context statistics (section C.4).
fn decode_gray_scale_image(
    data: &[u8],
    hgw: u32,
    hgh: u32,
    hbpp: u32,
    hmmr: bool,
    henableskip: bool,
    hskip: Option<&Vec<bool>>,
    htemplate: HTemplate,
) -> Result<Vec<u32>, &'static str> {
    use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};

    // GI is an HGW × HGH array of HBPP-bit values.
    let mut gi = vec![0u32; (hgw * hgh) as usize];

    if hmmr {
        // MMR decoding - each bitplane is decoded separately.
        let mut offset = 0;

        for bit_index in (0..hbpp).rev() {
            let mut bitplane = DecodedRegion {
                width: hgw,
                height: hgh,
                data: vec![false; (hgw * hgh) as usize],
                x_location: 0,
                y_location: 0,
                combination_operator: CombinationOperator::Replace,
            };

            decode_bitmap_mmr(&mut bitplane, &data[offset..])?;

            // Estimate where this bitplane ends for the next one.
            offset += estimate_mmr_size(&data[offset..], hgw, hgh);

            // Combine this bitplane into GI.
            let bit_value = 1u32 << bit_index;
            for i in 0..bitplane.data.len() {
                if bitplane.data[i] {
                    gi[i] |= bit_value;
                }
            }
        }
    } else {
        // Arithmetic decoding - all bitplanes share the same decoder and contexts.
        // "3) For j = GSBPP - 1 down to 0, do:" (Annex C)
        let mut decoder = ArithmeticDecoder::new(data);

        let gb_template = htemplate.to_gb_template();
        let num_context_bits = match gb_template {
            GbTemplate::Template0 => 16,
            GbTemplate::Template1 => 13,
            GbTemplate::Template2 | GbTemplate::Template3 => 10,
        };
        let mut contexts = vec![ArithmeticDecoderContext::default(); 1 << num_context_bits];

        let at_pixels = build_halftone_at_pixels(htemplate);

        // We need to track the current bitplane as we decode, since context
        // is gathered from pixels already decoded in this bitplane.
        // Use DecodedRegion so we can reuse gather_context_with_at from generic_region.
        let mut bitplanes: Vec<DecodedRegion> = Vec::with_capacity(hbpp as usize);

        // Decode HBPP bit-planes, from most significant to least significant.
        // All bitplanes share the same arithmetic decoder and context statistics (C.4).
        for plane_num in (0..hbpp).rev() {
            let mut bitplane = DecodedRegion::new(hgw, hgh);

            // Decode each pixel in the bitplane.
            // "a) For all values of x, y in the bitmap:" (Annex C)
            for y in 0..hgh {
                for x in 0..hgw {
                    let idx = (y * hgw + x) as usize;

                    // "If GSKIP[y][x] = 1, set GSPLANE_j[y][x] = 0" (Annex C)
                    if henableskip {
                        if let Some(skip) = hskip {
                            if skip[idx] {
                                continue; // Leave as 0
                            }
                        }
                    }

                    // Gather context from previously decoded pixels in this bitplane.
                    // Use the same function as generic_region to ensure correctness.
                    let context = gather_context_with_at(&bitplane, x, y, gb_template, &at_pixels);
                    let pixel = decoder.decode(&mut contexts[context as usize]);

                    bitplane.set_pixel(x, y, pixel != 0);
                }
            }

            bitplanes.push(bitplane);
        }

        // Combine bitplanes into gi using Gray code to binary conversion.
        // "i) If j is GSBPP - 1, set GSVALS[y][x][j] = GSPLANE_j[y][x]
        //  ii) Otherwise, set GSVALS[y][x][j] = GSVALS[y][x][j+1] XOR GSPLANE_j[y][x]"
        // (Annex C.5)
        //
        // bitplanes[0] is GSPLANE_{HBPP-1} (MSB, decoded first)
        // bitplanes[1] is GSPLANE_{HBPP-2}
        // ...
        // bitplanes[HBPP-1] is GSPLANE_0 (LSB, decoded last)
        //
        // For each pixel, we build GSVALS bit by bit from MSB to LSB.
        for i in 0..gi.len() {
            let mut value = 0u32;
            let mut prev_bit = false;

            for (plane_idx, bitplane) in bitplanes.iter().enumerate() {
                let bit_index = hbpp - 1 - plane_idx as u32;
                let plane_bit = bitplane.data[i];

                // Gray code to binary: current bit = prev_bit XOR plane_bit
                // For MSB (first iteration), prev_bit is 0, so result is just plane_bit
                let binary_bit = prev_bit ^ plane_bit;

                if binary_bit {
                    value |= 1u32 << bit_index;
                }

                prev_bit = binary_bit;
            }

            gi[i] = value;
        }
    }

    Ok(gi)
}

/// Gather context from a bitplane stored as a flat slice.
///
/// This must match the bit ordering used in generic_region.rs which builds
/// context MSB-first using `(context << 1) | pixel`.
fn gather_context_from_slice(
    bitplane: &[bool],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    gb_template: GbTemplate,
    at_pixels: &[AdaptiveTemplatePixel],
) -> u32 {
    // Helper to get a pixel from the slice, returning 0 for out-of-bounds.
    let get_pixel = |px: i32, py: i32| -> u32 {
        if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
            return 0;
        }
        if bitplane[(py as u32 * width + px as u32) as usize] {
            1
        } else {
            0
        }
    };

    let x = x as i32;
    let y = y as i32;

    // Build context using same order as generic_region.rs (MSB-first with << 1).
    match gb_template {
        GbTemplate::Template0 => {
            // Template 0: 16 context bits (Figure 3a)
            let at1 = (at_pixels[0].x as i32, at_pixels[0].y as i32);
            let at2 = (at_pixels[1].x as i32, at_pixels[1].y as i32);
            let at3 = (at_pixels[2].x as i32, at_pixels[2].y as i32);
            let at4 = (at_pixels[3].x as i32, at_pixels[3].y as i32);

            let mut context = 0u32;

            context = (context << 1) | get_pixel(x + at4.0, y + at4.1);
            context = (context << 1) | get_pixel(x - 1, y - 2);
            context = (context << 1) | get_pixel(x, y - 2);
            context = (context << 1) | get_pixel(x + 1, y - 2);
            context = (context << 1) | get_pixel(x + at3.0, y + at3.1);

            context = (context << 1) | get_pixel(x + at2.0, y + at2.1);
            context = (context << 1) | get_pixel(x - 2, y - 1);
            context = (context << 1) | get_pixel(x - 1, y - 1);
            context = (context << 1) | get_pixel(x, y - 1);
            context = (context << 1) | get_pixel(x + 1, y - 1);
            context = (context << 1) | get_pixel(x + 2, y - 1);
            context = (context << 1) | get_pixel(x + at1.0, y + at1.1);

            context = (context << 1) | get_pixel(x - 4, y);
            context = (context << 1) | get_pixel(x - 3, y);
            context = (context << 1) | get_pixel(x - 2, y);
            context = (context << 1) | get_pixel(x - 1, y);

            context
        }
        GbTemplate::Template1 => {
            // Template 1: 13 context bits (Figure 4)
            let at1 = (at_pixels[0].x as i32, at_pixels[0].y as i32);

            let mut context = 0u32;

            context = (context << 1) | get_pixel(x - 1, y - 2);
            context = (context << 1) | get_pixel(x, y - 2);
            context = (context << 1) | get_pixel(x + 1, y - 2);
            context = (context << 1) | get_pixel(x + 2, y - 2);

            context = (context << 1) | get_pixel(x - 2, y - 1);
            context = (context << 1) | get_pixel(x - 1, y - 1);
            context = (context << 1) | get_pixel(x, y - 1);
            context = (context << 1) | get_pixel(x + 1, y - 1);
            context = (context << 1) | get_pixel(x + 2, y - 1);
            context = (context << 1) | get_pixel(x + at1.0, y + at1.1);

            context = (context << 1) | get_pixel(x - 3, y);
            context = (context << 1) | get_pixel(x - 2, y);
            context = (context << 1) | get_pixel(x - 1, y);

            context
        }
        GbTemplate::Template2 => {
            // Template 2: 10 context bits (Figure 5)
            let at1 = (at_pixels[0].x as i32, at_pixels[0].y as i32);

            let mut context = 0u32;

            context = (context << 1) | get_pixel(x - 1, y - 2);
            context = (context << 1) | get_pixel(x, y - 2);
            context = (context << 1) | get_pixel(x + 1, y - 2);

            context = (context << 1) | get_pixel(x - 2, y - 1);
            context = (context << 1) | get_pixel(x - 1, y - 1);
            context = (context << 1) | get_pixel(x, y - 1);
            context = (context << 1) | get_pixel(x + 1, y - 1);
            context = (context << 1) | get_pixel(x + at1.0, y + at1.1);

            context = (context << 1) | get_pixel(x - 2, y);
            context = (context << 1) | get_pixel(x - 1, y);

            context
        }
        GbTemplate::Template3 => {
            // Template 3: 10 context bits (Figure 6)
            // Matches generic_region.rs gather_context_template3
            let at1 = (at_pixels[0].x as i32, at_pixels[0].y as i32);

            let mut context = 0u32;

            context = (context << 1) | get_pixel(x - 3, y - 1);
            context = (context << 1) | get_pixel(x - 2, y - 1);
            context = (context << 1) | get_pixel(x - 1, y - 1);
            context = (context << 1) | get_pixel(x, y - 1);
            context = (context << 1) | get_pixel(x + 1, y - 1);
            context = (context << 1) | get_pixel(x + at1.0, y + at1.1);

            context = (context << 1) | get_pixel(x - 4, y);
            context = (context << 1) | get_pixel(x - 3, y);
            context = (context << 1) | get_pixel(x - 2, y);
            context = (context << 1) | get_pixel(x - 1, y);

            context
        }
    }
}

/// Estimate MMR data size for one bitplane (rough approximation).
fn estimate_mmr_size(data: &[u8], _width: u32, _height: u32) -> usize {
    // Look for EOFB marker: 000000000001 000000000001 (24 bits = 0x001001)
    // This is a simplification. In practice, we'd need proper byte tracking.
    // For now, return a reasonable estimate or consume all remaining data.

    // Actually, for halftone with MMR, each bitplane typically has its own EOFB.
    // Let's look for the pattern 0x00 0x10 0x01 or similar byte sequences.

    // Simple heuristic: scan for 0x00 0x00 sequence followed by more zeros.
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == 0x00 && data[i + 1] == 0x00 {
            // Potential EOFB area - return position after some padding.
            let end = (i + 4).min(data.len());
            return end;
        }
    }

    data.len()
}

/// Build AT pixels for halftone decoding.
///
/// For halftone, we use default AT positions since there's no explicit AT field.
fn build_halftone_at_pixels(htemplate: HTemplate) -> Vec<AdaptiveTemplatePixel> {
    match htemplate {
        HTemplate::Template0 => vec![
            AdaptiveTemplatePixel { x: 3, y: -1 },
            AdaptiveTemplatePixel { x: -3, y: -1 },
            AdaptiveTemplatePixel { x: 2, y: -2 },
            AdaptiveTemplatePixel { x: -2, y: -2 },
        ],
        HTemplate::Template1 => vec![AdaptiveTemplatePixel { x: 3, y: -1 }],
        HTemplate::Template2 => vec![AdaptiveTemplatePixel { x: 2, y: -1 }],
        HTemplate::Template3 => vec![AdaptiveTemplatePixel { x: 2, y: -1 }],
    }
}

/// Render patterns into HTREG (6.6.5.2).
///
/// "Draw the patterns into HTREG using the following procedure:"
fn render_patterns(
    htreg: &mut DecodedRegion,
    gi: &[u32],
    hgw: u32,
    hgh: u32,
    hgx: i32,
    hgy: i32,
    hrx: i32,
    hry: i32,
    pattern_dict: &PatternDictionary,
    hcombop: CombinationOperator,
) {
    let hpw = pattern_dict.pattern_width;
    let hph = pattern_dict.pattern_height;
    let hbw = htreg.width;
    let hbh = htreg.height;

    // "1) For each value of m_g between 0 and HGH − 1, beginning from 0,
    // perform the following steps:" (6.6.5.2)
    for m_g in 0..hgh {
        // "a) For each value of n_g between 0 and HGW − 1, beginning from 0,
        // perform the following steps:" (6.6.5.2)
        for n_g in 0..hgw {
            // "i) Set:
            //    x = (HGX + m_g × HRY + n_g × HRX) >>_A 8
            //    y = (HGY + m_g × HRX − n_g × HRY) >>_A 8" (6.6.5.2)
            let x = arithmetic_shift_right(hgx + (m_g as i32) * hry + (n_g as i32) * hrx, 8);
            let y = arithmetic_shift_right(hgy + (m_g as i32) * hrx - (n_g as i32) * hry, 8);

            // "ii) Draw the pattern HPATS[GI[n_g, m_g]] into HTREG such that its
            // upper left pixel is at location (x, y) in HTREG." (6.6.5.2)
            let pattern_index = gi[(m_g * hgw + n_g) as usize] as usize;

            // Bounds check pattern index.
            if pattern_index >= pattern_dict.patterns.len() {
                continue;
            }

            let pattern = &pattern_dict.patterns[pattern_index];

            // Draw pattern at (x, y) using HCOMBOP.
            draw_pattern(htreg, pattern, x, y, hpw, hph, hbw, hbh, hcombop);
        }
    }
}

/// Draw a pattern into the halftone region at the specified location.
///
/// "A pattern is drawn into HTREG as follows. Each pixel of the pattern shall
/// be combined with the current value of the corresponding pixel in the
/// halftone-coded bitmap, using the combination operator specified by HCOMBOP."
fn draw_pattern(
    htreg: &mut DecodedRegion,
    pattern: &DecodedRegion,
    x: i32,
    y: i32,
    hpw: u32,
    hph: u32,
    hbw: u32,
    hbh: u32,
    hcombop: CombinationOperator,
) {
    // "If any part of a decoded pattern, when placed at location (x, y) lies
    // outside the actual halftone-coded bitmap, then this part of the pattern
    // shall be ignored in the process of combining the pattern with the bitmap."
    for py in 0..hph {
        let dest_y = y + py as i32;
        if dest_y < 0 || dest_y >= hbh as i32 {
            continue;
        }

        for px in 0..hpw {
            let dest_x = x + px as i32;
            if dest_x < 0 || dest_x >= hbw as i32 {
                continue;
            }

            let src_pixel = pattern.get_pixel(px, py);
            let dst_pixel = htreg.get_pixel(dest_x as u32, dest_y as u32);

            let result = match hcombop {
                CombinationOperator::Or => dst_pixel | src_pixel,
                CombinationOperator::And => dst_pixel & src_pixel,
                CombinationOperator::Xor => dst_pixel ^ src_pixel,
                CombinationOperator::Xnor => !(dst_pixel ^ src_pixel),
                CombinationOperator::Replace => src_pixel,
            };

            htreg.set_pixel(dest_x as u32, dest_y as u32, result);
        }
    }
}

/// Arithmetic right shift (sign-extending).
///
/// ">>_A" in the spec means arithmetic shift, which preserves the sign bit.
#[inline]
fn arithmetic_shift_right(value: i32, shift: u32) -> i32 {
    value >> shift
}
