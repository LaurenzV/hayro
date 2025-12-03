//! Reading raw JPEG2000 streams.

use crate::jp2::colr::{ColorSpace, ColorSpecificationBox, EnumeratedColorspace};
use crate::jp2::{DecodedImage, ImageBoxes};
use crate::{DecodeSettings, codestream};

pub(crate) fn read(data: &[u8], settings: &DecodeSettings) -> Result<DecodedImage, &'static str> {
    let decoded_codestream = codestream::read(data, settings)?;
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
