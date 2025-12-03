mod arithmetic_decoder;
mod bitplane;
mod build;
mod codestream;
mod decode;
mod idwt;
mod mct;
mod progression;
mod rect;
mod segment;
mod tag_tree;
mod tile;

use super::jp2::colr::{ColorSpace, ColorSpecificationBox, EnumeratedColorspace};
use super::jp2::{DecodedImage, ImageBoxes};
use crate::DecodeSettings;
use crate::j2c::codestream::markers;
use crate::reader::BitReader;

pub(crate) struct DecodedCodestream {
    /// The decoded components.
    pub(crate) components: Vec<ComponentData>,
    /// The width of the image.
    pub(crate) width: u32,
    /// The height of the image.
    pub(crate) height: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ComponentData {
    pub(crate) container: Vec<f32>,
    pub(crate) bit_depth: u8,
}

pub(crate) fn decode(data: &[u8], settings: &DecodeSettings) -> Result<DecodedImage, &'static str> {
    let decoded_codestream = read(data, settings)?;
    let mut boxes = ImageBoxes::default();

    // If we are just decoding a raw codestream, we assume greyscale or
    // RGB.
    let cs = if decoded_codestream.components.len() < 3 {
        ColorSpace::Enumerated(EnumeratedColorspace::Greyscale)
    } else {
        ColorSpace::Enumerated(EnumeratedColorspace::Srgb)
    };

    boxes.color_specification = Some(ColorSpecificationBox { color_space: cs });

    Ok(DecodedImage {
        decoded: decoded_codestream,
        boxes,
    })
}

pub(crate) fn read(
    stream: &[u8],
    settings: &DecodeSettings,
) -> Result<DecodedCodestream, &'static str> {
    let mut reader = BitReader::new(stream);

    let marker = reader.read_marker()?;
    if marker != markers::SOC {
        return Err("invalid marker: expected SOC marker");
    }

    let header = codestream::read_header(&mut reader, settings)?;
    let code_stream_data = reader
        .tail()
        .ok_or("code stream data is missing from image")?;
    let decoded = decode::decode(code_stream_data, &header)?;

    Ok(DecodedCodestream {
        width: header.size_data.image_width(),
        height: header.size_data.image_height(),
        components: decoded,
    })
}
