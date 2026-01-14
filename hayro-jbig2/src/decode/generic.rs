//! Generic region segment parsing and decoding (7.4.6, 6.2).

use alloc::vec;
use alloc::vec::Vec;

use super::{AdaptiveTemplatePixel, RegionSegmentInfo, Template, parse_region_segment_info};
use crate::arithmetic_decoder::{ArithmeticDecoder, Context};
use crate::bitmap::DecodedRegion;
use crate::error::{ParseError, RegionError, Result, TemplateError, bail};
use crate::reader::Reader;

/// Generic region decoding procedure (6.2).
pub(crate) fn decode(reader: &mut Reader<'_>, had_unknown_length: bool) -> Result<DecodedRegion> {
    let mut header = parse(reader)?;
    let mut encoded_data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    // "As a special case, as noted in 7.2.7, an immediate generic region segment
    // may have an unknown length. In this case, it also indicates the height of
    // the generic region (i.e. the number of rows that have been decoded in this
    // segment; it must be no greater than the region segment bitmap height value
    // in the segment's region segment information field." (7.4.6.4)
    if had_unknown_length {
        // Length has already been validated during segment parsing.
        let (head, tail) = encoded_data.split_at(encoded_data.len() - 4);
        let row_count = u32::from_be_bytes(tail.try_into().unwrap());

        if row_count > header.region_info.height {
            bail!(RegionError::InvalidDimension);
        }

        header.region_info.height = row_count;
        encoded_data = head;
    }

    let mut region = DecodedRegion {
        width: header.region_info.width,
        height: header.region_info.height,
        data: vec![false; (header.region_info.width * header.region_info.height) as usize],
        x_location: header.region_info.x_location,
        y_location: header.region_info.y_location,
        combination_operator: header.region_info.combination_operator,
    };

    if header.mmr {
        // "6.2.6 Decoding using MMR coding"
        let _ = decode_bitmap_mmr(&mut region, encoded_data)?;

        Ok(region)
    } else {
        // "6.2.5 Decoding using a template and arithmetic coding"
        decode_bitmap_arithmetic_coding(
            &mut region,
            encoded_data,
            header.template,
            header.tpgdon,
            &header.adaptive_template_pixels,
        )?;

        Ok(region)
    }
}

/// Parsed generic region segment header (7.4.6.1).
#[derive(Debug, Clone)]
struct GenericRegionHeader {
    region_info: RegionSegmentInfo,
    mmr: bool,
    template: Template,
    tpgdon: bool,
    adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,
}

/// Parse a generic region segment header (7.4.6.1).
fn parse(reader: &mut Reader<'_>) -> Result<GenericRegionHeader> {
    let region_info = parse_region_segment_info(reader)?;
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let mmr = flags & 0x01 != 0;
    let template = Template::from_byte(flags >> 1);
    let tpgdon = flags & 0x08 != 0;
    let ext_template = flags & 0x10 != 0;
    let adaptive_template_pixels = if mmr {
        Vec::new()
    } else {
        parse_adaptive_template_pixels(reader, template, ext_template)?
    };

    Ok(GenericRegionHeader {
        region_info,
        mmr,
        template,
        tpgdon,
        adaptive_template_pixels,
    })
}

/// Parse adaptive template pixel positions (7.4.6.3).
fn parse_adaptive_template_pixels(
    reader: &mut Reader<'_>,
    template: Template,
    // TODO: Find a test with this flag.
    _ext_template: bool,
) -> Result<Vec<AdaptiveTemplatePixel>> {
    let num_pixels = template.adaptive_template_pixels() as usize;

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

/// Decode a bitmap using MMR coding (6.2.6).
pub(crate) fn decode_bitmap_mmr(region: &mut DecodedRegion, data: &[u8]) -> Result<usize> {
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
        fn push_pixel(&mut self, white: bool) {
            if self.x < self.region.width {
                self.region.set_pixel(self.x, self.y, white);
                self.x += 1;
            }
        }

        fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
            let pixel_count = chunk_count as usize * 8;
            let start = (self.y * self.region.width + self.x) as usize;
            let end = (start + pixel_count).min(self.region.data.len());
            self.region.data[start..end].fill(white);
            self.x += pixel_count as u32;
        }

        fn next_line(&mut self) {
            self.x = 0;
            self.y += 1;
        }
    }

    let width = region.width;
    let height = region.height;
    let mut decoder = BitmapDecoder::new(region);

    let settings = hayro_ccitt::DecodeSettings {
        columns: width,
        rows: height,
        // "If the number of bytes contained in the encoded bitmap is known in
        // advance, then it is permissible for the data stream not to contain
        // an EOFB" (6.2.6). But it _can_ contain it, which is what this
        // flag indicates.
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

    // "An invocation of the generic region decoding procedure with MMR equal to
    // 1 shall consume an integral number of bytes, beginning and ending on a
    // byte boundary. This may involve skipping over some bits in the last byte
    // read." (6.2.6)
    //
    // hayro-ccitt already aligns to the byte boundary before returning, so
    // nothing else to do here.
    Ok(hayro_ccitt::decode(data, &mut decoder, &settings)
        .map_err(|_| RegionError::InvalidDimension)?)
}

/// Decode a bitmap using arithmetic coding (6.2.5).
pub(crate) fn decode_bitmap_arithmetic_coding(
    region: &mut DecodedRegion,
    data: &[u8],
    template: Template,
    tpgdon: bool,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
) -> Result<()> {
    let width = region.width;
    let height = region.height;

    let mut decoder = ArithmeticDecoder::new(data);

    let mut contexts = vec![Context::default(); 1 << template.context_bits()];

    // "1) Set: LTP = 0" (6.2.5.7)
    let mut ltp = false;

    // "3) Decode each row as follows:" (6.2.5.7)
    for y in 0..height {
        // "b) If TPGDON is 1, then decode a bit using the arithmetic entropy
        // coder" (6.2.5.7)
        if tpgdon {
            // See Figure 8 - 11.
            let sltp_context: u32 = match template {
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
                // If y == 0, pixels remain the same.
                if y > 0 {
                    let above = region.get_pixel(x, y - 1);
                    region.set_pixel(x, y, above);
                }
            }
        } else {
            // "d) If LTP = 0 then, from left to right, decode each pixel of the
            // current row of GBREG." (6.2.5.7)
            for x in 0..width {
                let context_bits = gather_context(region, x, y, template, adaptive_template_pixels);
                let pixel = decoder.decode(&mut contexts[context_bits as usize]);
                region.set_pixel(x, y, pixel != 0);
            }
        }
    }

    Ok(())
}

/// Gather context bits for a pixel at (x, y) (6.2.5.3, 6.2.5.4).
pub(crate) fn gather_context(
    region: &DecodedRegion,
    x: u32,
    y: u32,
    gb_template: Template,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
) -> u32 {
    match gb_template {
        // Context for Template 0 (Figure 3a, 16 pixels).
        Template::Template0 => {
            let x = x as i32;
            let y = y as i32;

            let at1 = (
                adaptive_template_pixels[0].x as i32,
                adaptive_template_pixels[0].y as i32,
            );
            let at2 = (
                adaptive_template_pixels[1].x as i32,
                adaptive_template_pixels[1].y as i32,
            );
            let at3 = (
                adaptive_template_pixels[2].x as i32,
                adaptive_template_pixels[2].y as i32,
            );
            let at4 = (
                adaptive_template_pixels[3].x as i32,
                adaptive_template_pixels[3].y as i32,
            );

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
        // Context for Template 1 (Figure 4).
        Template::Template1 => {
            let x = x as i32;
            let y = y as i32;

            let at1 = (
                adaptive_template_pixels[0].x as i32,
                adaptive_template_pixels[0].y as i32,
            );

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
        // Context for Template 2 (Figure 5).
        Template::Template2 => {
            let x = x as i32;
            let y = y as i32;

            let at1 = (
                adaptive_template_pixels[0].x as i32,
                adaptive_template_pixels[0].y as i32,
            );

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
        // Context for Template 3 (Figure 6).
        Template::Template3 => {
            let x = x as i32;
            let y = y as i32;

            let at1 = (
                adaptive_template_pixels[0].x as i32,
                adaptive_template_pixels[0].y as i32,
            );

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
    }
}

// TODO: Rewrite everything below & make it more performant. Also don't
// cast from u32 to i32.

/// Get a pixel value, returning 0 for out-of-bounds coordinates.
#[inline]
pub(crate) fn get_pixel(region: &DecodedRegion, x: i32, y: i32) -> u32 {
    // "Near the edges of the bitmap, these neighbour references might not lie in
    // the actual bitmap. The rule to satisfy out-of-bounds references shall be:
    // All pixels lying outside the bounds of the actual bitmap have the value 0."
    // (6.2.5.2)
    if x < 0 || y < 0 || x >= region.width as i32 {
        0
    } else if region.get_pixel(x as u32, y as u32) {
        1
    } else {
        0
    }
}
