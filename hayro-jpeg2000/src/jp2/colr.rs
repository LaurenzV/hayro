use crate::jp2::icc::ICCMetadata;
use crate::reader::BitReader;

/// Parse the Color specification box (colr), defined in I.5.3.3.
pub(crate) fn parse(data: &[u8]) -> Option<ColorSpecificationBox> {
    if data.len() < 3 {
        return None;
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
        _ => return None,
    };

    Some(ColorSpecificationBox {
        method,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct ColorSpecificationBox {
    pub(crate) method: ColorSpace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ColorSpace {
    Enumerated(EnumeratedColorspace),
    Icc(Vec<u8>),
}

impl ColorSpace {
    pub(crate) fn expected_number_of_channels(&self) -> u8 {
        match self {
            ColorSpace::Enumerated(e) => e.expected_number_of_channels(),
            ColorSpace::Icc(i) => {
                ICCMetadata::from_data(i)
                    .map(|d| d.color_space.num_components())
                    // Let's just assume RGB. There is one OpenJPEG test
                    // case that decodes differently than OpenJPEG does
                    // if we don't do that (OpenJPEG interprets the image
                    // as RGB + alpha, while if we just look at the channel
                    // count we would infer CMYK instead.
                    .unwrap_or(3)
            }
        }
    }
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

    /// Returns the number of colour channels this enumerated space expects without accounting
    /// for extra alpha channels.
    pub fn expected_number_of_channels(&self) -> u8 {
        match self {
            EnumeratedColorspace::BiLevel1 => 1,
            EnumeratedColorspace::YCbCr1 => 3,
            EnumeratedColorspace::YCbCr2 => 3,
            EnumeratedColorspace::YCbCr3 => 3,
            EnumeratedColorspace::PhotoYcc => 3,
            EnumeratedColorspace::Cmy => 3,
            EnumeratedColorspace::Cmyk => 4,
            EnumeratedColorspace::Ycck => 4,
            EnumeratedColorspace::CieLab => 3,
            EnumeratedColorspace::BiLevel2 => 1,
            EnumeratedColorspace::Srgb => 3,
            EnumeratedColorspace::Greyscale => 1,
            EnumeratedColorspace::Sycc => 3,
            EnumeratedColorspace::CieJab => 3,
            EnumeratedColorspace::EsRgb => 3,
            EnumeratedColorspace::RommRgb => 3,
            EnumeratedColorspace::YPbPr112560 => 3,
            EnumeratedColorspace::YPbPr125050 => 3,
            EnumeratedColorspace::EsYcc => 3,
            EnumeratedColorspace::ScRgb => 3,
            EnumeratedColorspace::ScRgbGray => 1,
        }
    }
}