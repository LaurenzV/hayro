//! Generic refinement region segment parsing and decoding (7.4.7, 6.3).

use alloc::vec;
use alloc::vec::Vec;

use super::{
    AdaptiveTemplatePixel, RefinementTemplate, RegionSegmentInfo, parse_refinement_at_pixels,
    parse_region_segment_info,
};
use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::decode::generic::get_pixel;
use crate::error::{DecodeError, ParseError, RegionError, Result, bail};
use crate::reader::Reader;

/// Generic refinement region decoding procedure (6.3).
pub(crate) fn decode(reader: &mut Reader<'_>, reference: &DecodedRegion) -> Result<DecodedRegion> {
    let header = parse(reader)?;

    // Validate that the region fits within the reference bitmap.
    // When referring to another segment, dimensions must match exactly (7.4.7.5).
    // When using the page bitmap as reference, the region must fit within the page.
    if header.region_info.width > reference.width || header.region_info.height > reference.height {
        bail!(RegionError::InvalidDimension);
    }

    let reference_dx = i32::try_from(reference.x_location)
        .ok()
        .and_then(|r| {
            i32::try_from(header.region_info.x_location)
                .ok()
                .and_then(|h| r.checked_sub(h))
        })
        .ok_or(DecodeError::Overflow)?;
    let reference_dy = i32::try_from(reference.y_location)
        .ok()
        .and_then(|r| {
            i32::try_from(header.region_info.y_location)
                .ok()
                .and_then(|h| r.checked_sub(h))
        })
        .ok_or(DecodeError::Overflow)?;
    let encoded_data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    let mut decoder = ArithmeticDecoder::new(encoded_data);
    let num_context_bits = header.template.context_bits();
    let mut contexts = vec![Context::default(); 1 << num_context_bits];

    let width = header.region_info.width;
    let height = header.region_info.height;

    let mut region = DecodedRegion {
        width,
        height,
        data: vec![false; (width * height) as usize],
        x_location: header.region_info.x_location,
        y_location: header.region_info.y_location,
        combination_operator: header.region_info.combination_operator,
    };

    decode_bitmap(
        &mut decoder,
        &mut contexts,
        &mut region,
        reference,
        reference_dx,
        reference_dy,
        header.template,
        &header.adaptive_template_pixels,
        header.tpgron,
    )?;

    Ok(region)
}

/// Parsed generic refinement region segment header (7.4.7.1).
#[derive(Debug, Clone)]
struct GenericRefinementRegionHeader {
    region_info: RegionSegmentInfo,
    template: RefinementTemplate,
    tpgron: bool,
    adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,
}

/// Parse a generic refinement region segment header (7.4.7.1).
fn parse(reader: &mut Reader<'_>) -> Result<GenericRefinementRegionHeader> {
    let region_info = parse_region_segment_info(reader)?;
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let template = RefinementTemplate::from_byte(flags);
    let tpgron = flags & 0x02 != 0;
    let adaptive_template_pixels = if template == RefinementTemplate::Template0 {
        parse_refinement_at_pixels(reader)?
    } else {
        Vec::new()
    };

    Ok(GenericRefinementRegionHeader {
        region_info,
        template,
        tpgron,
        adaptive_template_pixels,
    })
}

/// Decode a refinement bitmap (6.3.5.6).
pub(crate) fn decode_bitmap(
    // TODO: Maybe reduce number of arguments?
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [Context],
    region: &mut DecodedRegion,
    reference: &DecodedRegion,
    reference_dx: i32,
    reference_dy: i32,
    gr_template: RefinementTemplate,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
    tpgron: bool,
) -> Result<()> {
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
            let sltp_context: u32 = match gr_template {
                RefinementTemplate::Template0 => 0b0000000010000,
                RefinementTemplate::Template1 => 0b0000001000,
            };
            let sltp = decoder.decode(&mut contexts[sltp_context as usize]);
            // "Let SLTP be the value of this bit. Set: LTP = LTP XOR SLTP"
            ltp = ltp != (sltp != 0);
        }

        let mut decode_single =
            |x: u32, decoder: &mut ArithmeticDecoder<'_>, region: &mut DecodedRegion| {
                let context = gather_context(
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
            };

        // "c) If LTP = 0 then, from left to right, explicitly decode all pixels
        // of the current row of GRREG." (6.3.5.6)
        if !ltp {
            for x in 0..width {
                decode_single(x, decoder, region);
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
                let tpgrpix = tpgron && {
                    let ref_x = x as i32 - reference_dx;
                    let ref_y = y as i32 - reference_dy;

                    let mut all_same = true;

                    let center = get_pixel(reference, ref_x, ref_y);

                    // Check all 9 pixels in the 3×3 region.
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            if get_pixel(reference, ref_x + dx, ref_y + dy) != center {
                                all_same = false;
                                break;
                            }
                        }
                    }

                    all_same
                };

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
                    region.set_pixel(x, y, tpgrval != 0);
                } else {
                    // "iii) Otherwise, explicitly decode the current pixel using the
                    // methodology of steps 3 c) i) through 3 c) iii) above." (6.3.5.6)
                    decode_single(x, decoder, region);
                }
            }
        }
    }

    Ok(())
}

/// Gather context bits for refinement decoding (6.3.5.3).
fn gather_context(
    region: &DecodedRegion,
    reference: &DecodedRegion,
    x: u32,
    y: u32,
    reference_dx: i32,
    reference_dy: i32,
    gr_template: RefinementTemplate,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
) -> u32 {
    let x = x as i32;
    let y = y as i32;

    // Reference bitmap coordinates.
    let ref_x = x - reference_dx;
    let ref_y = y - reference_dy;

    match gr_template {
        // Context for Template 0 (Figure 12).
        RefinementTemplate::Template0 => {
            // 13-pixel template with 2 AT pixels.
            let at1 = adaptive_template_pixels[0];
            let at2 = adaptive_template_pixels[1];

            let mut context = 0_u32;

            // 4 pixels from the bitmap we are currently decoding.
            context = (context << 1) | get_pixel(region, x + at1.x as i32, y + at1.y as i32);
            context = (context << 1) | get_pixel(region, x, y - 1);
            context = (context << 1) | get_pixel(region, x + 1, y - 1);
            context = (context << 1) | get_pixel(region, x - 1, y);

            // 9 pixels from the reference bitmap.
            context =
                (context << 1) | get_pixel(reference, ref_x + at2.x as i32, ref_y + at2.y as i32);
            context = (context << 1) | get_pixel(reference, ref_x, ref_y - 1);
            context = (context << 1) | get_pixel(reference, ref_x + 1, ref_y - 1);
            context = (context << 1) | get_pixel(reference, ref_x - 1, ref_y);
            context = (context << 1) | get_pixel(reference, ref_x, ref_y);
            context = (context << 1) | get_pixel(reference, ref_x + 1, ref_y);
            context = (context << 1) | get_pixel(reference, ref_x - 1, ref_y + 1);
            context = (context << 1) | get_pixel(reference, ref_x, ref_y + 1);
            context = (context << 1) | get_pixel(reference, ref_x + 1, ref_y + 1);

            context
        }
        // Context for Template 1 (Figure 13).
        RefinementTemplate::Template1 => {
            // 10-pixel template.
            let mut context = 0_u32;

            // 4 pixels from the bitmap we are currently decoding.
            context = (context << 1) | get_pixel(region, x - 1, y - 1);
            context = (context << 1) | get_pixel(region, x, y - 1);
            context = (context << 1) | get_pixel(region, x + 1, y - 1);
            context = (context << 1) | get_pixel(region, x - 1, y);

            // 6 pixels from the reference bitmap.
            context = (context << 1) | get_pixel(reference, ref_x, ref_y - 1);
            context = (context << 1) | get_pixel(reference, ref_x - 1, ref_y);
            context = (context << 1) | get_pixel(reference, ref_x, ref_y);
            context = (context << 1) | get_pixel(reference, ref_x + 1, ref_y);
            context = (context << 1) | get_pixel(reference, ref_x, ref_y + 1);
            context = (context << 1) | get_pixel(reference, ref_x + 1, ref_y + 1);

            context
        }
    }
}
