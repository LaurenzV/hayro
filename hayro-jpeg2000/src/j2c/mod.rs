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
pub(crate) mod simd;
mod tag_tree;
mod tile;

use super::jp2::ImageBoxes;
use super::jp2::colr::{ColorSpace, ColorSpecificationBox, EnumeratedColorspace};
use crate::j2c::codestream::markers;
use crate::reader::BitReader;
use crate::{DecodeSettings, Image, resolve_alpha_and_color_space};

pub(crate) use codestream::Header;
pub(crate) use decode::decode;

pub(crate) struct ParsedCodestream<'a> {
    pub(crate) header: Header<'a>,
    pub(crate) data: &'a [u8],
}

pub(crate) struct DecodedCodestream {
    /// The decoded components.
    pub(crate) components: Vec<ComponentData>,
}

#[derive(Debug, Clone)]
pub(crate) struct ComponentData {
    pub(crate) container: Vec<f32>,
    pub(crate) bit_depth: u8,
}

pub(crate) fn parse<'a>(
    stream: &'a [u8],
    settings: &DecodeSettings,
) -> Result<Image<'a>, &'static str> {
    let parsed_codestream = parse_raw(stream, settings)?;
    let header = &parsed_codestream.header;
    let mut boxes = ImageBoxes::default();

    // If we are just decoding a raw codestream, we assume greyscale or
    // RGB.
    let cs = if header.component_infos.len() < 3 {
        ColorSpace::Enumerated(EnumeratedColorspace::Greyscale)
    } else {
        ColorSpace::Enumerated(EnumeratedColorspace::Srgb)
    };

    boxes.color_specification = Some(ColorSpecificationBox { color_space: cs });

    let (color_space, has_alpha) =
        resolve_alpha_and_color_space(&boxes, &parsed_codestream.header, settings)?;

    Ok(Image {
        codestream: parsed_codestream.data,
        header: parsed_codestream.header,
        boxes,
        settings: *settings,
        color_space,
        has_alpha,
    })
}

pub(crate) fn parse_raw<'a>(
    stream: &'a [u8],
    settings: &DecodeSettings,
) -> Result<ParsedCodestream<'a>, &'static str> {
    let mut reader = BitReader::new(stream);

    let marker = reader.read_marker()?;
    if marker != markers::SOC {
        return Err("invalid marker: expected SOC marker");
    }

    let header = codestream::read_header(&mut reader, settings)?;
    let code_stream_data = reader
        .tail()
        .ok_or("code stream data is missing from image")?;

    Ok(ParsedCodestream {
        header,
        data: code_stream_data,
    })
}
