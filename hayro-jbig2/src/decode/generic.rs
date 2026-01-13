//! Generic region segment parsing and decoding (7.4.6, 6.2).

use alloc::vec;
use alloc::vec::Vec;

use super::{AdaptiveTemplatePixel, RegionSegmentInfo, Template, parse_region_segment_info};
use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::error::{DecodeError, ParseError, RegionError, Result, TemplateError, bail};
use crate::reader::Reader;

/// Generic region decoding procedure (6.2).
///
/// "This decoding procedure is used to decode a rectangular array of 0 or 1
/// values, which are coded one pixel at a time (i.e., it is used to decode a
/// bitmap using simple, generic, coding)." (6.2.1)
///
/// "The data parts of all three of the generic region segment types
/// ('intermediate generic region', 'immediate generic region' and 'immediate
/// lossless generic region') are coded identically, but are acted upon
/// differently, see 8.2." (7.4.6)
///
/// If `had_unknown_length` is true, the segment data ends with a row count
/// field that should be used instead of the height from the region info.
///
/// Returns the decoded region with its location and combination operator.
pub(crate) fn decode(reader: &mut Reader<'_>, had_unknown_length: bool) -> Result<DecodedRegion> {
    let mut header = parse(reader)?;

    // Get the remaining data after the header for decoding.
    let mut encoded_data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    // "As a special case, as noted in 7.2.7, an immediate generic region segment
    // may have an unknown length. In this case, it also indicates the height of
    // the generic region (i.e. the number of rows that have been decoded in this
    // segment; it must be no greater than the region segment bitmap height value
    // in the segment's region segment information field." (7.4.6.4)
    if had_unknown_length {
        // Length has already been validated during segment parsing.
        let row_count_bytes = &encoded_data[encoded_data.len() - 4..];
        let row_count = u32::from_be_bytes(row_count_bytes.try_into().unwrap());

        if row_count > header.region_info.height {
            bail!(RegionError::InvalidDimension);
        }

        header.region_info.height = row_count;
        encoded_data = &encoded_data[..encoded_data.len() - 4];
    }

    // Decode the region.
    if header.mmr {
        // "6.2.6 Decoding using MMR coding"
        decode_generic_region_mmr(&header, encoded_data)
    } else {
        // "6.2.5 Decoding using a template and arithmetic coding"
        decode_generic_region_ad(&header, encoded_data)
    }
}

/// Parsed generic region segment header (7.4.6.1).
#[derive(Debug, Clone)]
struct GenericRegionHeader {
    /// Region segment information field (7.4.1).
    region_info: RegionSegmentInfo,
    /// "Bit 0: MMR" (7.4.6.2)
    mmr: bool,
    /// "Bits 1-2: GBTEMPLATE. This field specifies the template used for
    /// template-based arithmetic coding. If MMR is 1 then this field must
    /// contain the value zero." (7.4.6.2)
    gb_template: Template,
    /// "Bit 3: TPGDON. This field specifies whether typical prediction for
    /// generic direct coding is used." (7.4.6.2)
    tpgdon: bool,
    /// "Bit 4: EXTTEMPLATE. This field specifies whether extended reference
    /// template is used." (7.4.6.2)
    _ext_template: bool,
    /// Adaptive template pixels (7.4.6.3).
    ///
    /// "This field is only present if MMR is 0."
    /// - If GBTEMPLATE is 0 and EXTTEMPLATE is 0: 4 AT pixels (8 bytes)
    /// - If GBTEMPLATE is 0 and EXTTEMPLATE is 1: 12 AT pixels (24 bytes)
    /// - If GBTEMPLATE is 1, 2, or 3: 1 AT pixel (2 bytes)
    adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,
}

/// Parse a generic region segment header (7.4.6.1).
fn parse(reader: &mut Reader<'_>) -> Result<GenericRegionHeader> {
    // 7.4.6.1: "The data part of a generic region segment begins with a generic
    // region segment data header. This header contains the fields shown in
    // Figure 47."

    // Region segment information field (7.4.1)
    let region_info = parse_region_segment_info(reader)?;

    // 7.4.6.2: Generic region segment flags
    // "This one-byte field is formatted as shown in Figure 48."
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;

    // "Bit 0: MMR"
    let mmr = flags & 0x01 != 0;

    // "Bits 1-2: GBTEMPLATE. This field specifies the template used for
    // template-based arithmetic coding. If MMR is 1 then this field must
    // contain the value zero."
    let gb_template = Template::from_byte(flags >> 1);

    // "Bit 3: TPGDON. This field specifies whether typical prediction for
    // generic direct coding is used."
    let tpgdon = flags & 0x08 != 0;

    // "Bit 4: EXTTEMPLATE. This field specifies whether extended reference
    // template is used."
    let ext_template = flags & 0x10 != 0;

    // "Bits 5-7: Reserved; must be zero."
    if flags & 0xE0 != 0 {
        bail!(TemplateError::Invalid);
    }

    // Validate MMR + GBTEMPLATE constraint
    if mmr && gb_template != Template::Template0 {
        bail!(TemplateError::Invalid);
    }

    // 7.4.6.3: Generic region segment AT flags
    // "This field is only present if MMR is 0."
    let adaptive_template_pixels = if mmr {
        Vec::new()
    } else {
        parse_adaptive_template_pixels(reader, gb_template, ext_template)?
    };

    Ok(GenericRegionHeader {
        region_info,
        mmr,
        gb_template,
        tpgdon,
        _ext_template: ext_template,
        adaptive_template_pixels,
    })
}

/// Parse adaptive template pixel positions (7.4.6.3).
fn parse_adaptive_template_pixels(
    reader: &mut Reader<'_>,
    gb_template: Template,
    ext_template: bool,
) -> Result<Vec<AdaptiveTemplatePixel>> {
    // "If GBTEMPLATE is 0 and EXTTEMPLATE is 0, it is an eight-byte field,
    // formatted as shown in Figure 49."
    //
    // "If GBTEMPLATE is 0 and EXTTEMPLATE is 1, it is a 32-byte field,
    // formatted as shown in Figure 50." (but we only use first 24 bytes
    // for 12 AT pixels)
    //
    // "If GBTEMPLATE is 1, 2 or 3, it is a two-byte field formatted as shown
    // in Figure 51."

    let num_pixels = match gb_template {
        Template::Template0 => {
            if ext_template {
                bail!(DecodeError::Unsupported);
            } else {
                4
            }
        }
        Template::Template1 | Template::Template2 | Template::Template3 => 1,
    };

    let mut pixels = Vec::with_capacity(num_pixels);

    for _ in 0..num_pixels {
        let x = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;
        let y = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;

        // Validate AT pixel location (6.2.5.4, Figure 7).
        // AT pixels must reference already-decoded pixels:
        // - y must be <= 0 (current row or above)
        // - if y == 0, x must be < 0 (strictly to the left of current pixel)
        if y > 0 || (y == 0 && x >= 0) {
            bail!(TemplateError::InvalidAtPixel);
        }

        pixels.push(AdaptiveTemplatePixel { x, y });
    }

    Ok(pixels)
}

/// Decode a generic region using MMR coding (6.2.6).
fn decode_generic_region_mmr(header: &GenericRegionHeader, data: &[u8]) -> Result<DecodedRegion> {
    if !header.mmr {
        bail!(TemplateError::Invalid);
    }

    let mut region = DecodedRegion {
        width: header.region_info.width,
        height: header.region_info.height,
        data: vec![false; (header.region_info.width * header.region_info.height) as usize],
        x_location: header.region_info.x_location,
        y_location: header.region_info.y_location,
        combination_operator: header.region_info.combination_operator,
    };

    let _ = decode_bitmap_mmr(&mut region, data)?;
    Ok(region)
}

/// Decode a bitmap using MMR coding (6.2.6).
///
/// "If MMR is 1, the generic region decoding procedure is identical to an
/// MMR (Modified Modified READ) decoder described in Recommendation ITU-T
/// T.6 (G4)." (6.2.6)
///
/// The region must already have width, height, and data allocated.
/// Returns the number of bytes consumed from the input data.
pub(crate) fn decode_bitmap_mmr(region: &mut DecodedRegion, data: &[u8]) -> Result<usize> {
    let width = region.width;
    let height = region.height;

    let mut decoder = BitmapDecoder::new(region);

    // "An invocation of the generic region decoding procedure with MMR equal to
    // 1 shall consume an integral number of bytes, beginning and ending on a
    // byte boundary. This may involve skipping over some bits in the last byte
    // read." (6.2.6)
    //
    // "If the number of bytes contained in the encoded bitmap is known in
    // advance, then it is permissible for the data stream not to contain an
    // EOFB (000000000001000000000001) at the end of the MMR-encoded data."
    // (6.2.6)
    let settings = hayro_ccitt::DecodeSettings {
        columns: width,
        rows: height,
        // "If the number of bytes contained in the encoded bitmap is known in
        // advance, then it is permissible for the data stream not to contain
        // an EOFB" (but it _can_) (6.2.6)
        //
        end_of_block: true,
        end_of_line: false,
        rows_are_byte_aligned: false,
        encoding: hayro_ccitt::EncodingMode::Group4,
        // "Pixels decoded by the MMR decoder having the value 'black' shall be
        // treated as having the value 1. Pixels decoded by the MMR decoder
        // having the value 'white' shall be treated as having the value 0."
        // (6.2.6)
        //
        // hayro-ccitt uses 1 for white, 0 for black by default, so we need to
        // invert to match JBIG2 convention.
        invert_black: true,
    };

    Ok(hayro_ccitt::decode(data, &mut decoder, &settings)
        .map_err(|_| RegionError::InvalidDimension)?)
}

/// A decoder sink that writes decoded pixels into a `DecodedRegion`.
struct BitmapDecoder<'a> {
    region: &'a mut DecodedRegion,
    x: u32,
    y: u32,
}

impl<'a> BitmapDecoder<'a> {
    fn new(region: &'a mut DecodedRegion) -> Self {
        Self { region, x: 0, y: 0 }
    }
}

impl hayro_ccitt::Decoder for BitmapDecoder<'_> {
    /// Push a single pixel with the given color.
    fn push_pixel(&mut self, white: bool) {
        if self.x < self.region.width {
            self.region.set_pixel(self.x, self.y, white);
            self.x += 1;
        }
    }

    /// Push multiple chunks of 8 pixels of the same color.
    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        let pixel_count = chunk_count as usize * 8;
        let start = (self.y * self.region.width + self.x) as usize;
        let end = (start + pixel_count).min(self.region.data.len());
        self.region.data[start..end].fill(white);
        self.x += pixel_count as u32;
    }

    /// Called when a row has been completed.
    fn next_line(&mut self) {
        self.x = 0;
        self.y += 1;
    }
}

/// Decode a generic region using arithmetic coding (6.2.5).
///
/// "If MMR is 0 the generic region decoding procedure is based on arithmetic
/// coding with a template to determine the coding state." (6.2.5.1)
fn decode_generic_region_ad(header: &GenericRegionHeader, data: &[u8]) -> Result<DecodedRegion> {
    let mut region = DecodedRegion {
        width: header.region_info.width,
        height: header.region_info.height,
        data: vec![false; (header.region_info.width * header.region_info.height) as usize],
        x_location: header.region_info.x_location,
        y_location: header.region_info.y_location,
        combination_operator: header.region_info.combination_operator,
    };

    decode_bitmap_arith(
        &mut region,
        data,
        header.gb_template,
        header.tpgdon,
        &header.adaptive_template_pixels,
    )?;
    Ok(region)
}

/// Decode a bitmap using arithmetic coding (6.2.5).
///
/// "If MMR is 0 the generic region decoding procedure is based on arithmetic
/// coding with a template to determine the coding state." (6.2.5.1)
///
/// The region must already have width, height, and data allocated.
pub(crate) fn decode_bitmap_arith(
    region: &mut DecodedRegion,
    data: &[u8],
    gb_template: Template,
    tpgdon: bool,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
) -> Result<()> {
    let width = region.width;
    let height = region.height;

    let mut decoder = ArithmeticDecoder::new(data);

    let mut contexts = vec![Context::default(); 1 << gb_template.context_bits()];

    // "1) Set: LTP = 0" (6.2.5.7)
    let mut ltp = false;

    // "3) Decode each row as follows:" (6.2.5.7)
    for y in 0..height {
        // "b) If TPGDON is 1, then decode a bit using the arithmetic entropy
        // coder" (6.2.5.7)
        if tpgdon {
            // See Figure 8 - 11.
            let sltp_context: u32 = match gb_template {
                Template::Template0 => 0b1001101100100101,
                Template::Template1 => 0b0011110010101,
                Template::Template2 => 0b0011100101,
                Template::Template3 => 0b0110010101,
            };
            let sltp = decoder.decode(&mut contexts[sltp_context as usize]);
            // "Let SLTP be the value of this bit. Set: LTP = LTP XOR SLTP" (6.2.5.7)
            ltp = ltp != (sltp != 0);
        }

        // "c) If LTP = 1 then set every pixel of the current row of GBREG equal
        // to the corresponding pixel of the row immediately above." (6.2.5.7)
        if ltp {
            for x in 0..width {
                if y > 0 {
                    let above = region.get_pixel(x, y - 1);
                    region.set_pixel(x, y, above);
                }
                // If y == 0, pixels remain 0 (default)
            }
        } else {
            // "d) If LTP = 0 then, from left to right, decode each pixel of the
            // current row of GBREG." (6.2.5.7)
            for x in 0..width {
                let context_bits =
                    gather_context_with_at(region, x, y, gb_template, adaptive_template_pixels);
                let pixel = decoder.decode(&mut contexts[context_bits as usize]);
                region.set_pixel(x, y, pixel != 0);
            }
        }
    }

    Ok(())
}

/// Gather context bits for a pixel at (x, y) (6.2.5.3, 6.2.5.4).
///
/// "Form an integer CONTEXT by gathering the values of the image pixels overlaid
/// by the template (including AT pixels) at its current location." (6.2.5.7)
pub(crate) fn gather_context_with_at(
    region: &DecodedRegion,
    x: u32,
    y: u32,
    gb_template: Template,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
) -> u32 {
    match gb_template {
        Template::Template0 => {
            gather_context_template0_no_ext(region, x, y, adaptive_template_pixels)
        }
        Template::Template1 => gather_context_template1(region, x, y, adaptive_template_pixels),
        Template::Template2 => gather_context_template2(region, x, y, adaptive_template_pixels),
        Template::Template3 => gather_context_template3(region, x, y, adaptive_template_pixels),
    }
}

/// Get a pixel value, returning 0 for out-of-bounds coordinates.
///
/// "Near the edges of the bitmap, these neighbour references might not lie in
/// the actual bitmap. The rule to satisfy out-of-bounds references shall be:
/// All pixels lying outside the bounds of the actual bitmap have the value 0."
/// (6.2.5.2)
#[inline]
fn get_pixel(region: &DecodedRegion, x: i32, y: i32) -> u32 {
    // Note: y >= region.height is not checked because all template positions
    // have y <= 0 relative to the current pixel (6.2.5.4, Figure 7).
    if x < 0 || y < 0 || x >= region.width as i32 {
        0
    } else if region.get_pixel(x as u32, y as u32) {
        1
    } else {
        0
    }
}

/// Gather context for Template 0 (Figure 3a, 16 pixels).
fn gather_context_template0_no_ext(
    region: &DecodedRegion,
    x: u32,
    y: u32,
    at: &[AdaptiveTemplatePixel],
) -> u32 {
    let x = x as i32;
    let y = y as i32;

    let at1 = (at[0].x as i32, at[0].y as i32);
    let at2 = (at[1].x as i32, at[1].y as i32);
    let at3 = (at[2].x as i32, at[2].y as i32);
    let at4 = (at[3].x as i32, at[3].y as i32);

    let mut context = 0_u32;

    context = (context << 1) | get_pixel(region, x + at4.0, y + at4.1);
    context = (context << 1) | get_pixel(region, x - 1, y - 2);
    context = (context << 1) | get_pixel(region, x, y - 2);
    context = (context << 1) | get_pixel(region, x + 1, y - 2);
    context = (context << 1) | get_pixel(region, x + at3.0, y + at3.1);

    context = (context << 1) | get_pixel(region, x + at2.0, y + at2.1);
    context = (context << 1) | get_pixel(region, x - 2, y - 1);
    context = (context << 1) | get_pixel(region, x - 1, y - 1);
    context = (context << 1) | get_pixel(region, x, y - 1);
    context = (context << 1) | get_pixel(region, x + 1, y - 1);
    context = (context << 1) | get_pixel(region, x + 2, y - 1);
    context = (context << 1) | get_pixel(region, x + at1.0, y + at1.1);

    context = (context << 1) | get_pixel(region, x - 4, y);
    context = (context << 1) | get_pixel(region, x - 3, y);
    context = (context << 1) | get_pixel(region, x - 2, y);
    context = (context << 1) | get_pixel(region, x - 1, y);

    context
}

/// Gather context for Template 1 (Figure 4).
fn gather_context_template1(
    region: &DecodedRegion,
    x: u32,
    y: u32,
    at: &[AdaptiveTemplatePixel],
) -> u32 {
    let x = x as i32;
    let y = y as i32;

    let at1 = (at[0].x as i32, at[0].y as i32);

    let mut context = 0_u32;

    context = (context << 1) | get_pixel(region, x - 1, y - 2);
    context = (context << 1) | get_pixel(region, x, y - 2);
    context = (context << 1) | get_pixel(region, x + 1, y - 2);
    context = (context << 1) | get_pixel(region, x + 2, y - 2);

    context = (context << 1) | get_pixel(region, x - 2, y - 1);
    context = (context << 1) | get_pixel(region, x - 1, y - 1);
    context = (context << 1) | get_pixel(region, x, y - 1);
    context = (context << 1) | get_pixel(region, x + 1, y - 1);
    context = (context << 1) | get_pixel(region, x + 2, y - 1);
    context = (context << 1) | get_pixel(region, x + at1.0, y + at1.1);

    context = (context << 1) | get_pixel(region, x - 3, y);
    context = (context << 1) | get_pixel(region, x - 2, y);
    context = (context << 1) | get_pixel(region, x - 1, y);

    context
}

/// Gather context for Template 2 (Figure 5).
fn gather_context_template2(
    region: &DecodedRegion,
    x: u32,
    y: u32,
    at: &[AdaptiveTemplatePixel],
) -> u32 {
    let x = x as i32;
    let y = y as i32;

    let at1 = (at[0].x as i32, at[0].y as i32);

    let mut context = 0_u32;

    context = (context << 1) | get_pixel(region, x - 1, y - 2);
    context = (context << 1) | get_pixel(region, x, y - 2);
    context = (context << 1) | get_pixel(region, x + 1, y - 2);

    context = (context << 1) | get_pixel(region, x - 2, y - 1);
    context = (context << 1) | get_pixel(region, x - 1, y - 1);
    context = (context << 1) | get_pixel(region, x, y - 1);
    context = (context << 1) | get_pixel(region, x + 1, y - 1);
    context = (context << 1) | get_pixel(region, x + at1.0, y + at1.1);

    context = (context << 1) | get_pixel(region, x - 2, y);
    context = (context << 1) | get_pixel(region, x - 1, y);

    context
}

/// Gather context for Template 3 (Figure 6).
fn gather_context_template3(
    region: &DecodedRegion,
    x: u32,
    y: u32,
    at: &[AdaptiveTemplatePixel],
) -> u32 {
    let x = x as i32;
    let y = y as i32;

    let at1 = (at[0].x as i32, at[0].y as i32);

    let mut context = 0_u32;

    context = (context << 1) | get_pixel(region, x - 3, y - 1);
    context = (context << 1) | get_pixel(region, x - 2, y - 1);
    context = (context << 1) | get_pixel(region, x - 1, y - 1);
    context = (context << 1) | get_pixel(region, x, y - 1);
    context = (context << 1) | get_pixel(region, x + 1, y - 1);
    context = (context << 1) | get_pixel(region, x + at1.0, y + at1.1);

    context = (context << 1) | get_pixel(region, x - 4, y);
    context = (context << 1) | get_pixel(region, x - 3, y);
    context = (context << 1) | get_pixel(region, x - 2, y);
    context = (context << 1) | get_pixel(region, x - 1, y);

    context
}
