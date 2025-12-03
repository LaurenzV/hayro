//! The color specification box (colr), defined in I.5.3.3.

use crate::jp2::ImageBoxes;
use crate::reader::BitReader;

pub(crate) fn parse(boxes: &mut ImageBoxes, data: &[u8]) -> Option<()> {
    let mut reader = BitReader::new(data);

    let meth = reader.read_byte()?;
    // We don't care about those.
    let _prec = reader.read_byte()?;
    let _approx = reader.read_byte()?;

    let method = match meth {
        1 => {
            let enumerated = reader.read_u32()?;
            ColorSpace::Enumerated(EnumeratedColorspace::from_raw(enumerated)?)
        }
        2 => {
            let profile_data = reader.tail()?.to_vec();
            ColorSpace::Icc(profile_data)
        }
        _ => ColorSpace::Unknown,
    };

    boxes.color_specification = Some(ColorSpecificationBox {
        color_space: method,
    });

    Some(())
}

#[derive(Debug, Clone)]
pub(crate) struct ColorSpecificationBox {
    pub(crate) color_space: ColorSpace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ColorSpace {
    Enumerated(EnumeratedColorspace),
    Icc(Vec<u8>),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumeratedColorspace {
    BiLevel1,
    YCbCr1,
    YCbCr2,
    YCbCr3,
    PhotoYcc,
    Cmy,
    Cmyk,
    Ycck,
    CieLab,
    BiLevel2,
    Srgb,
    Greyscale,
    Sycc,
    CieJab,
    EsRgb,
    RommRgb,
    YPbPr112560,
    YPbPr125050,
    EsYcc,
    ScRgb,
    ScRgbGray,
}

impl EnumeratedColorspace {
    fn from_raw(value: u32) -> Option<Self> {
        match value {
            0 => Some(EnumeratedColorspace::BiLevel1),
            1 => Some(EnumeratedColorspace::YCbCr1),
            3 => Some(EnumeratedColorspace::YCbCr2),
            4 => Some(EnumeratedColorspace::YCbCr3),
            9 => Some(EnumeratedColorspace::PhotoYcc),
            11 => Some(EnumeratedColorspace::Cmy),
            12 => Some(EnumeratedColorspace::Cmyk),
            13 => Some(EnumeratedColorspace::Ycck),
            14 => Some(EnumeratedColorspace::CieLab),
            15 => Some(EnumeratedColorspace::BiLevel2),
            16 => Some(EnumeratedColorspace::Srgb),
            17 => Some(EnumeratedColorspace::Greyscale),
            18 => Some(EnumeratedColorspace::Sycc),
            19 => Some(EnumeratedColorspace::CieJab),
            20 => Some(EnumeratedColorspace::EsRgb),
            21 => Some(EnumeratedColorspace::RommRgb),
            22 => Some(EnumeratedColorspace::YPbPr112560),
            23 => Some(EnumeratedColorspace::YPbPr125050),
            24 => Some(EnumeratedColorspace::EsYcc),
            25 => Some(EnumeratedColorspace::ScRgb),
            26 => Some(EnumeratedColorspace::ScRgbGray),
            _ => None,
        }
    }
}
