//! Generic refinement region segment parsing and decoding (7.4.7, 6.3).

use super::{RegionSegmentInfo, parse_region_segment_info};
use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::reader::Reader;

/// Adaptive template pixel position for refinement regions.
///
/// "The AT coordinate X and Y fields are signed values, and may take on values
/// that are permitted according to 6.3.5.3." (7.4.7.3)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RefinementAdaptiveTemplatePixel {
    pub x: i8,
    pub y: i8,
}

/// Template used for refinement arithmetic coding (7.4.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrTemplate {
    /// Template 0: 13 pixels (6.3.5.2, Figure 12)
    Template0 = 0,
    /// Template 1: 10 pixels (6.3.5.2, Figure 13)
    Template1 = 1,
}

/// Parsed generic refinement region segment header (7.4.7.1).
#[derive(Debug, Clone)]
pub(crate) struct GenericRefinementRegionHeader {
    /// Region segment information field (7.4.1).
    pub region_info: RegionSegmentInfo,
    /// "Bit 0: GRTEMPLATE. This field specifies the template used for
    /// template-based arithmetic coding." (7.4.7.2)
    pub gr_template: GrTemplate,
    /// "Bit 1: TPGRON. This field specifies whether typical prediction for
    /// generic refinement is used." (7.4.7.2)
    pub tpgron: bool,
    /// Adaptive template pixels (7.4.7.3).
    ///
    /// "This field is only present if GRTEMPLATE is 0."
    /// Contains 2 AT pixels (4 bytes): GRATX1, GRATY1, GRATX2, GRATY2
    pub adaptive_template_pixels: Vec<RefinementAdaptiveTemplatePixel>,
}

/// Parse a generic refinement region segment header (7.4.7.1).
pub(crate) fn parse_generic_refinement_region_header(
    reader: &mut Reader<'_>,
) -> Result<GenericRefinementRegionHeader, &'static str> {
    // 7.4.7.1: "The data part of a generic refinement region segment begins
    // with a generic refinement region segment data header. This header
    // contains the fields shown in Figure 52."

    // Region segment information field (7.4.1)
    let region_info = parse_region_segment_info(reader)?;

    // 7.4.7.2: Generic refinement region segment flags
    // "This one-byte field is formatted as shown in Figure 53."
    let flags = reader.read_byte().ok_or("unexpected end of data")?;

    // "Bit 0: GRTEMPLATE"
    let gr_template = if flags & 0x01 == 0 {
        GrTemplate::Template0
    } else {
        GrTemplate::Template1
    };

    // "Bit 1: TPGRON"
    let tpgron = flags & 0x02 != 0;

    // 7.4.7.3: Generic refinement region segment AT flags
    // "This field is only present if GRTEMPLATE is 0."
    let adaptive_template_pixels = if gr_template == GrTemplate::Template0 {
        parse_refinement_adaptive_template_pixels(reader)?
    } else {
        Vec::new()
    };

    Ok(GenericRefinementRegionHeader {
        region_info,
        gr_template,
        tpgron,
        adaptive_template_pixels,
    })
}

/// Parse refinement adaptive template pixel positions (7.4.7.3).
///
/// "It is a four-byte field, formatted as shown in Figure 54."
fn parse_refinement_adaptive_template_pixels(
    reader: &mut Reader<'_>,
) -> Result<Vec<RefinementAdaptiveTemplatePixel>, &'static str> {
    let mut pixels = Vec::with_capacity(2);

    // GRATX1, GRATY1
    let x1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y1 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x1, y: y1 });

    // GRATX2, GRATY2
    let x2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    let y2 = reader.read_byte().ok_or("unexpected end of data")? as i8;
    pixels.push(RefinementAdaptiveTemplatePixel { x: x2, y: y2 });

    Ok(pixels)
}

/// Generic refinement region decoding procedure (6.3).
///
/// "This decoding procedure is used to decode a rectangular array of 0 or 1
/// values, which are coded one pixel at a time. There is a reference bitmap
/// known to the decoding procedure, and this is used as part of the decoding
/// process. The reference bitmap is intended to resemble the bitmap being
/// decoded, and this similarity is used to increase compression." (6.3.1)
pub(crate) fn decode_generic_refinement_region(
    reader: &mut Reader<'_>,
    reference: &DecodedRegion,
) -> Result<DecodedRegion, &'static str> {
    let header = parse_generic_refinement_region_header(reader)?;

    // Validate that the region fits within the reference bitmap.
    // When referring to another segment, dimensions must match exactly (7.4.7.5).
    // When using the page bitmap as reference, the region must fit within the page.
    if header.region_info.width > reference.width || header.region_info.height > reference.height {
        return Err("refinement region dimensions exceed reference");
    }

    // "The X offset of the reference bitmap with respect to the bitmap
    // being decoded." (Table 6, GRREFERENCEDX/GRREFERENCEDY)
    //
    // The offset is computed from the difference in location between the
    // reference and the region being decoded.
    let reference_dx = reference.x_location as i32 - header.region_info.x_location as i32;
    let reference_dy = reference.y_location as i32 - header.region_info.y_location as i32;

    let encoded_data = reader.tail().ok_or("unexpected end of data")?;

    decode_refinement_bitmap(&header, encoded_data, reference, reference_dx, reference_dy)
}

/// Decode the refinement bitmap (6.3.5.6).
///
/// "The decoding of the bitmap proceeds as follows:" (6.3.5.6)
fn decode_refinement_bitmap(
    header: &GenericRefinementRegionHeader,
    data: &[u8],
    reference: &DecodedRegion,
    reference_dx: i32,
    reference_dy: i32,
) -> Result<DecodedRegion, &'static str> {
    let mut decoder = ArithmeticDecoder::new(data);

    let num_context_bits = match header.gr_template {
        GrTemplate::Template0 => 13,
        GrTemplate::Template1 => 10,
    };
    let mut contexts = vec![Context::default(); 1 << num_context_bits];

    let width = header.region_info.width;
    let height = header.region_info.height;

    // "2) Create a bitmap GRREG of width GRW and height GRH pixels." (6.3.5.6)
    let mut region = DecodedRegion {
        width,
        height,
        data: vec![false; (width * height) as usize],
        x_location: header.region_info.x_location,
        y_location: header.region_info.y_location,
        combination_operator: header.region_info.combination_operator,
    };

    decode_refinement_bitmap_with(
        &mut decoder,
        &mut contexts,
        &mut region,
        reference,
        reference_dx,
        reference_dy,
        header.gr_template,
        &header.adaptive_template_pixels,
        header.tpgron,
    )?;

    Ok(region)
}

/// Decode a refinement bitmap with provided decoder and contexts.
///
/// This is the core refinement decoding loop (6.3.5.6). It allows sharing
/// decoder and context state across multiple refinements (e.g., in symbol
/// dictionary decoding per Table 18).
pub(crate) fn decode_refinement_bitmap_with(
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [Context],
    region: &mut DecodedRegion,
    reference: &DecodedRegion,
    reference_dx: i32,
    reference_dy: i32,
    gr_template: GrTemplate,
    adaptive_template_pixels: &[RefinementAdaptiveTemplatePixel],
    tpgron: bool,
) -> Result<(), &'static str> {
    let width = region.width;
    let height = region.height;

    // "1) Set LTP = 0." (6.3.5.6)
    let mut ltp = false;

    // "3) Decode each row as follows:" (6.3.5.6)
    for y in 0..height {
        // "b) If TPGRON is 1, then decode a bit using the arithmetic entropy
        // coder" (6.3.5.6)
        if tpgron {
            // Context for SLTP depends on template (Figures 14, 15).
            // The SLTP context has only the center reference pixel (0,0) set.
            let sltp_context: u32 = match gr_template {
                GrTemplate::Template0 => 0b0000000010000,
                GrTemplate::Template1 => 0b0000001000,
            };
            let sltp = decoder.decode(&mut contexts[sltp_context as usize]);
            // "Let SLTP be the value of this bit. Set: LTP = LTP XOR SLTP"
            ltp = ltp != (sltp != 0);
        }

        // "c) If LTP = 0 then, from left to right, explicitly decode all pixels
        // of the current row of GRREG." (6.3.5.6)
        if !ltp {
            for x in 0..width {
                let context = gather_refinement_context(
                    region,
                    reference,
                    x,
                    y,
                    reference_dx,
                    reference_dy,
                    gr_template,
                    adaptive_template_pixels,
                );
                let pixel = decoder.decode(&mut contexts[context as usize]);
                region.set_pixel(x, y, pixel != 0);
            }
        } else {
            // "d) If LTP = 1 then, from left to right, implicitly decode certain
            // pixels of the current row of GRREG, and explicitly decode the rest."
            // (6.3.5.6)
            for x in 0..width {
                // "i) Set TPGRPIX equal to 1 if:
                //    - TPGRON is 1 AND;
                //    - a 3 × 3 pixel array in the reference bitmap (Figure 16),
                //      centred at the location corresponding to the current pixel,
                //      contains pixels all of the same value." (6.3.5.6)
                let tpgrpix =
                    tpgron && is_tpgr(reference, x as i32 - reference_dx, y as i32 - reference_dy);

                if tpgrpix {
                    // "ii) If TPGRPIX is 1 then implicitly decode the current pixel
                    // by setting it equal to its predicted value (TPGRVAL)." (6.3.5.6)
                    //
                    // "When TPGRPIX is set to 1, set TPGRVAL equal to the current pixel
                    // predicted value, which is the common value of the nine adjacent
                    // pixels in the 3 × 3 array." (6.3.5.6)
                    let ref_x = x as i32 - reference_dx;
                    let ref_y = y as i32 - reference_dy;
                    let tpgrval = get_pixel(reference, ref_x, ref_y);
                    region.set_pixel(x, y, tpgrval);
                } else {
                    // "iii) Otherwise, explicitly decode the current pixel using the
                    // methodology of steps 3 c) i) through 3 c) iii) above." (6.3.5.6)
                    let context = gather_refinement_context(
                        region,
                        reference,
                        x,
                        y,
                        reference_dx,
                        reference_dy,
                        gr_template,
                        adaptive_template_pixels,
                    );
                    let pixel = decoder.decode(&mut contexts[context as usize]);
                    region.set_pixel(x, y, pixel != 0);
                }
            }
        }
    }

    Ok(())
}

/// Check the TPGR condition (Figure 16).
///
/// Returns true if all 9 pixels in the 3×3 region centered at (`ref_x`, `ref_y`)
/// in the reference bitmap have the same value.
fn is_tpgr(reference: &DecodedRegion, ref_x: i32, ref_y: i32) -> bool {
    // Get the center pixel value.
    let center = get_pixel(reference, ref_x, ref_y);

    // Check all 9 pixels in the 3×3 region (Figure 16).
    for dy in -1..=1 {
        for dx in -1..=1 {
            if get_pixel(reference, ref_x + dx, ref_y + dy) != center {
                return false;
            }
        }
    }

    true
}

/// Get a pixel from a region, returning false for out-of-bounds.
///
/// "Near the edges of the bitmap, these neighbour references might not lie in
/// the actual bitmap. The rule to satisfy out-of-bounds references shall be:
/// All pixels lying outside the bounds of the actual bitmap or the reference
/// bitmap have the value 0." (6.3.5.2)
#[inline]
fn get_pixel(region: &DecodedRegion, x: i32, y: i32) -> bool {
    if x < 0 || y < 0 || x >= region.width as i32 || y >= region.height as i32 {
        false
    } else {
        region.get_pixel(x as u32, y as u32)
    }
}

/// Gather context bits for refinement decoding (6.3.5.3).
///
/// "The values of the pixels in the template shall be combined to form a
/// context." (6.3.5.3)
fn gather_refinement_context(
    region: &DecodedRegion,
    reference: &DecodedRegion,
    x: u32,
    y: u32,
    reference_dx: i32,
    reference_dy: i32,
    gr_template: GrTemplate,
    adaptive_template_pixels: &[RefinementAdaptiveTemplatePixel],
) -> u32 {
    let x = x as i32;
    let y = y as i32;

    // Reference bitmap coordinates.
    let ref_x = x - reference_dx;
    let ref_y = y - reference_dy;

    match gr_template {
        GrTemplate::Template0 => {
            // Figure 12: 13-pixel template with 2 AT pixels.
            // Left group (bitmap being decoded): 4 pixels (including RA1)
            // Right group (reference bitmap): 9 pixels (including RA2)
            let at1 = adaptive_template_pixels[0]; // RA1 for decoded bitmap
            let at2 = adaptive_template_pixels[1]; // RA2 for reference bitmap

            let mut context = 0_u32;

            context = (context << 1) | get_pixel_u32(region, x + at1.x as i32, y + at1.y as i32);
            context = (context << 1) | get_pixel_u32(region, x, y - 1);
            context = (context << 1) | get_pixel_u32(region, x + 1, y - 1);
            context = (context << 1) | get_pixel_u32(region, x - 1, y);

            context = (context << 1)
                | get_pixel_u32(reference, ref_x + at2.x as i32, ref_y + at2.y as i32);
            context = (context << 1) | get_pixel_u32(reference, ref_x, ref_y - 1);
            context = (context << 1) | get_pixel_u32(reference, ref_x + 1, ref_y - 1);
            context = (context << 1) | get_pixel_u32(reference, ref_x - 1, ref_y);
            context = (context << 1) | get_pixel_u32(reference, ref_x, ref_y);
            context = (context << 1) | get_pixel_u32(reference, ref_x + 1, ref_y);
            context = (context << 1) | get_pixel_u32(reference, ref_x - 1, ref_y + 1);
            context = (context << 1) | get_pixel_u32(reference, ref_x, ref_y + 1);
            context = (context << 1) | get_pixel_u32(reference, ref_x + 1, ref_y + 1);

            context
        }
        GrTemplate::Template1 => {
            let mut context = 0_u32;

            context = (context << 1) | get_pixel_u32(region, x - 1, y - 1);
            context = (context << 1) | get_pixel_u32(region, x, y - 1);
            context = (context << 1) | get_pixel_u32(region, x + 1, y - 1);
            context = (context << 1) | get_pixel_u32(region, x - 1, y);

            context = (context << 1) | get_pixel_u32(reference, ref_x, ref_y - 1);
            context = (context << 1) | get_pixel_u32(reference, ref_x - 1, ref_y);
            context = (context << 1) | get_pixel_u32(reference, ref_x, ref_y);
            context = (context << 1) | get_pixel_u32(reference, ref_x + 1, ref_y);
            context = (context << 1) | get_pixel_u32(reference, ref_x, ref_y + 1);
            context = (context << 1) | get_pixel_u32(reference, ref_x + 1, ref_y + 1);

            context
        }
    }
}

/// Get a pixel as u32, returning 0 for out-of-bounds.
#[inline]
fn get_pixel_u32(region: &DecodedRegion, x: i32, y: i32) -> u32 {
    u32::from(get_pixel(region, x, y))
}
