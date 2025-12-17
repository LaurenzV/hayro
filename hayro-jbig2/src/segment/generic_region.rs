//! Generic region segment parsing and decoding (7.4.6, 6.2).
//!
//! "The data parts of all three of the generic region segment types
//! ('intermediate generic region', 'immediate generic region' and 'immediate
//! lossless generic region') are coded identically, but are acted upon
//! differently, see 8.2." (7.4.6)

use crate::DecodeContext;
use crate::bitmap::Bitmap;
use crate::reader::Reader;
use crate::segment::region::{RegionSegmentInfo, parse_region_segment_info};

/// Adaptive template pixel position.
///
/// "The AT coordinate X and Y fields are signed values." (7.4.6.3)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AdaptiveTemplatePixel {
    pub x: i8,
    pub y: i8,
}

/// Parsed generic region segment header (7.4.6.1).
#[derive(Debug, Clone)]
pub(crate) struct GenericRegionHeader {
    /// Region segment information field (7.4.1).
    pub region_info: RegionSegmentInfo,
    /// "Bit 0: MMR" (7.4.6.2)
    pub mmr: bool,
    /// "Bits 1-2: GBTEMPLATE. This field specifies the template used for
    /// template-based arithmetic coding. If MMR is 1 then this field must
    /// contain the value zero." (7.4.6.2)
    pub gb_template: u8,
    /// "Bit 3: TPGDON. This field specifies whether typical prediction for
    /// generic direct coding is used." (7.4.6.2)
    pub tpgdon: bool,
    /// "Bit 4: EXTTEMPLATE. This field specifies whether extended reference
    /// template is used." (7.4.6.2)
    pub ext_template: bool,
    /// Adaptive template pixels (7.4.6.3).
    ///
    /// "This field is only present if MMR is 0."
    /// - If GBTEMPLATE is 0 and EXTTEMPLATE is 0: 4 AT pixels (8 bytes)
    /// - If GBTEMPLATE is 0 and EXTTEMPLATE is 1: 12 AT pixels (24 bytes)
    /// - If GBTEMPLATE is 1, 2, or 3: 1 AT pixel (2 bytes)
    pub adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,
}

/// Parse a generic region segment header (7.4.6.1).
pub(crate) fn parse_generic_region_header(
    reader: &mut Reader<'_>,
) -> Result<GenericRegionHeader, &'static str> {
    // 7.4.6.1: "The data part of a generic region segment begins with a generic
    // region segment data header. This header contains the fields shown in
    // Figure 47."

    // Region segment information field (7.4.1)
    let region_info = parse_region_segment_info(reader)?;

    // 7.4.6.2: Generic region segment flags
    // "This one-byte field is formatted as shown in Figure 48."
    let flags = reader.read_byte().ok_or("unexpected end of data")?;

    // "Bit 0: MMR"
    let mmr = flags & 0x01 != 0;

    // "Bits 1-2: GBTEMPLATE. This field specifies the template used for
    // template-based arithmetic coding. If MMR is 1 then this field must
    // contain the value zero."
    let gb_template = (flags >> 1) & 0x03;

    // "Bit 3: TPGDON. This field specifies whether typical prediction for
    // generic direct coding is used."
    let tpgdon = flags & 0x08 != 0;

    // "Bit 4: EXTTEMPLATE. This field specifies whether extended reference
    // template is used."
    let ext_template = flags & 0x10 != 0;

    // "Bits 5-7: Reserved; must be zero."
    if flags & 0xE0 != 0 {
        return Err("reserved bits in generic region segment flags must be 0");
    }

    // Validate MMR + GBTEMPLATE constraint
    if mmr && gb_template != 0 {
        return Err("GBTEMPLATE must be 0 when MMR is 1");
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
        ext_template,
        adaptive_template_pixels,
    })
}

/// Parse adaptive template pixel positions (7.4.6.3).
fn parse_adaptive_template_pixels(
    reader: &mut Reader<'_>,
    gb_template: u8,
    ext_template: bool,
) -> Result<Vec<AdaptiveTemplatePixel>, &'static str> {
    // "If GBTEMPLATE is 0 and EXTTEMPLATE is 0, it is an eight-byte field,
    // formatted as shown in Figure 49."
    //
    // "If GBTEMPLATE is 0 and EXTTEMPLATE is 1, it is a 32-byte field,
    // formatted as shown in Figure 50." (but we only use first 24 bytes
    // for 12 AT pixels)
    //
    // "If GBTEMPLATE is 1, 2 or 3, it is a two-byte field formatted as shown
    // in Figure 51."

    let num_pixels = if gb_template == 0 {
        if ext_template { 12 } else { 4 }
    } else {
        1
    };

    let mut pixels = Vec::with_capacity(num_pixels);

    for _ in 0..num_pixels {
        let x = reader.read_byte().ok_or("unexpected end of data")? as i8;
        let y = reader.read_byte().ok_or("unexpected end of data")? as i8;
        pixels.push(AdaptiveTemplatePixel { x, y });
    }

    Ok(pixels)
}

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
pub(crate) fn decode_generic_region(
    ctx: &mut DecodeContext,
    reader: &mut Reader<'_>,
) -> Result<(), &'static str> {
    let header = parse_generic_region_header(reader)?;

    // Get the remaining data after the header for decoding.
    let encoded_data = reader.tail().ok_or("unexpected end of data")?;

    // Decode the region.
    let region = if header.mmr {
        // "6.2.6 Decoding using MMR coding"
        decode_generic_region_mmr(&header, encoded_data)?
    } else {
        // Arithmetic coding not yet implemented.
        return Err("arithmetic coding not yet implemented");
    };

    // "These operators describe how the segment's bitmap is to be combined
    // with the page bitmap." (7.4.1.5)
    ctx.page_bitmap.combine(
        &region,
        header.region_info.x_location,
        header.region_info.y_location,
        header.region_info.combination_operator,
    );

    Ok(())
}

/// Decode a generic region using MMR coding (6.2.6).
fn decode_generic_region_mmr(
    header: &GenericRegionHeader,
    data: &[u8],
) -> Result<Bitmap, &'static str> {
    // "If MMR is 1, the generic region decoding procedure is identical to an
    // MMR (Modified Modified READ) decoder described in Recommendation ITU-T
    // T.6 (G4)." (6.2.6)
    if !header.mmr {
        return Err("decode_generic_region_mmr called with MMR=0");
    }

    let width = header.region_info.width;
    let height = header.region_info.height;

    // "2) Create a bitmap GBREG of width GBW and height GBH pixels." (6.2.5.7)
    let mut bitmap = Bitmap::new(width, height);

    // Create a decoder that writes into our bitmap.
    let mut decoder = BitmapDecoder::new(&mut bitmap);

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
        // an EOFB" (6.2.6)
        //
        // We know the byte count from the segment data length, so EOFB is
        // optional. We set end_of_block to false to decode based on row count.
        end_of_block: false,
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

    hayro_ccitt::decode(data, &mut decoder, &settings).ok_or("MMR decoding failed")?;

    Ok(bitmap)
}

/// A decoder sink that writes decoded pixels into a Bitmap.
struct BitmapDecoder<'a> {
    bitmap: &'a mut Bitmap,
    x: u32,
    y: u32,
}

impl<'a> BitmapDecoder<'a> {
    fn new(bitmap: &'a mut Bitmap) -> Self {
        Self { bitmap, x: 0, y: 0 }
    }
}

impl hayro_ccitt::Decoder for BitmapDecoder<'_> {
    /// "Push a single packed byte containing the data for 8 pixels."
    fn push_byte(&mut self, byte: u8) {
        // Write 8 pixels from the byte (MSB first).
        for i in 0..8 {
            if self.x >= self.bitmap.width {
                break;
            }
            let bit = (byte >> (7 - i)) & 1;
            self.bitmap.set_pixel(self.x, self.y, bit != 0);
            self.x += 1;
        }
    }

    /// "Push multiple columns of same-color pixels."
    fn push_bytes(&mut self, byte: u8, count: usize) {
        for _ in 0..count {
            self.push_byte(byte);
        }
    }

    /// "Called when a row has been completed."
    fn next_line(&mut self) {
        self.x = 0;
        self.y += 1;
    }
}
