//! The color specification box (colr), defined in I.5.3.3.

use crate::jp2::ImageBoxes;
use crate::reader::BitReader;

pub(crate) fn parse(boxes: &mut ImageBoxes, data: &[u8]) -> Option<()> {
    if boxes.color_specification.is_some() {
        // "A JP2 file may contain multiple Colour Specification boxes, but
        // must contain at least one, specifying different methods
        // for achieving "equivalent" results. A conforming JP2 reader shall
        // ignore all Colour Specification boxes after the first.
        // However, readers conforming to other standards may use those boxes as
        // defined in those other standards."

        return Some(());
    }

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
            0 => Some(Self::BiLevel1),
            1 => Some(Self::YCbCr1),
            3 => Some(Self::YCbCr2),
            4 => Some(Self::YCbCr3),
            9 => Some(Self::PhotoYcc),
            11 => Some(Self::Cmy),
            12 => Some(Self::Cmyk),
            13 => Some(Self::Ycck),
            14 => Some(Self::CieLab),
            15 => Some(Self::BiLevel2),
            16 => Some(Self::Srgb),
            17 => Some(Self::Greyscale),
            18 => Some(Self::Sycc),
            19 => Some(Self::CieJab),
            20 => Some(Self::EsRgb),
            21 => Some(Self::RommRgb),
            22 => Some(Self::YPbPr112560),
            23 => Some(Self::YPbPr125050),
            24 => Some(Self::EsYcc),
            25 => Some(Self::ScRgb),
            26 => Some(Self::ScRgbGray),
            _ => None,
        }
    }
}
