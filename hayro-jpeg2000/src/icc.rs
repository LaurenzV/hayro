#[derive(Clone, Hash, Debug, Eq, PartialEq)]
pub(crate) enum ICCColorSpace {
    Xyz,
    Lab,
    Luv,
    Ycbr,
    Yxy,
    Lms,
    Rgb,
    Gray,
    Hsv,
    Hls,
    Cmyk,
    Cmy,
    OneClr,
    ThreeClr,
    FourClr,
    // There are more, but those should be the most important
    // ones.
}

impl ICCColorSpace {
    pub(crate) fn num_components(&self) -> u8 {
        match self {
            ICCColorSpace::Xyz => 3,
            ICCColorSpace::Lab => 3,
            ICCColorSpace::Luv => 3,
            ICCColorSpace::Ycbr => 3,
            ICCColorSpace::Yxy => 3,
            ICCColorSpace::Lms => 3,
            ICCColorSpace::Rgb => 3,
            ICCColorSpace::Gray => 1,
            ICCColorSpace::Hsv => 3,
            ICCColorSpace::Hls => 3,
            ICCColorSpace::Cmyk => 4,
            ICCColorSpace::Cmy => 3,
            ICCColorSpace::OneClr => 1,
            ICCColorSpace::ThreeClr => 3,
            ICCColorSpace::FourClr => 4,
        }
    }
}

impl TryFrom<u32> for ICCColorSpace {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x58595A20 => Ok(ICCColorSpace::Xyz),
            0x4C616220 => Ok(ICCColorSpace::Lab),
            0x4C757620 => Ok(ICCColorSpace::Luv),
            0x59436272 => Ok(ICCColorSpace::Ycbr),
            0x59787920 => Ok(ICCColorSpace::Yxy),
            0x4C4D5320 => Ok(ICCColorSpace::Lms),
            0x52474220 => Ok(ICCColorSpace::Rgb),
            0x47524159 => Ok(ICCColorSpace::Gray),
            0x48535620 => Ok(ICCColorSpace::Hsv),
            0x484C5320 => Ok(ICCColorSpace::Hls),
            0x434D594B => Ok(ICCColorSpace::Cmyk),
            0x434D5920 => Ok(ICCColorSpace::Cmy),
            0x31434C52 => Ok(ICCColorSpace::OneClr),
            0x33434C52 => Ok(ICCColorSpace::ThreeClr),
            0x34434C52 => Ok(ICCColorSpace::FourClr),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Hash, Debug, Eq, PartialEq)]
pub(crate) struct ICCMetadata {
    pub(crate) color_space: ICCColorSpace,
}

impl ICCMetadata {
    pub(crate) fn from_data(data: &[u8]) -> Option<Self> {
        let color_space = {
            let marker = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
            ICCColorSpace::try_from(marker).ok()?
        };

        Some(Self { color_space })
    }
}
