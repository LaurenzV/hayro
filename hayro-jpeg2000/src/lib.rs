use crate::bitmap::Bitmap;
use crate::boxes::{
    CHANNEL_DEFINITION, COLOUR_SPECIFICATION, CONTIGUOUS_CODESTREAM, FILE_TYPE, IMAGE_HEADER,
    JP2_HEADER, JP2_SIGNATURE, read_box, tag_to_string,
};
use hayro_common::byte::Reader;
use log::{debug, warn};

mod arithmetic_decoder;
pub mod bitmap;
pub(crate) mod bitplane;
pub mod boxes;
mod codestream;
mod decode;
pub(crate) mod idwt;
mod progression;
pub(crate) mod rect;
mod tag_tree;
mod tile;

/// Image metadata extracted from JP2 Header box.
#[derive(Debug, Clone)]
pub struct ImageMetadata {
    /// Image area height in reference grid points.
    pub height: u32,
    /// Image area width in reference grid points.
    pub width: u32,
    /// Intellectual property flag (0 = no IPR box, 1 = contains IPR box).
    pub has_intellectual_property: u8,
    /// Colour specification information from the Colour Specification box.
    pub colour_specification: Option<ColourSpecification>,
    /// Channel definitions specified by the Channel Definition box (cdef).
    pub channel_definitions: Vec<ChannelDefinition>,
}

/// Parsed contents of a Colour Specification box as defined in ISO/IEC 15444-1.
#[derive(Debug, Clone)]
pub struct ColourSpecification {
    /// METH, the specification method for this box.
    pub method: ColourSpecificationMethod,
    /// PREC, precedence hint.
    pub precedence: u8,
    /// APPROX, colourspace approximation indicator.
    pub approximation: u8,
}

/// The way the colourspace is described inside the Colour Specification box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColourSpecificationMethod {
    /// Enumerated colourspace (EnumCS field present).
    Enumerated(EnumeratedColourspace),
    /// ICC profile (PROFILE field present).
    IccProfile(Vec<u8>),
    /// Reserved or unsupported method; stores raw value for debugging.
    Unknown(u8),
}

/// Enumerated colourspace identifiers defined by ISO/IEC 15444-1 Table L.10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumeratedColourspace {
    Srgb,
    Greyscale,
    Sycc,
    Reserved(u32),
}

impl EnumeratedColourspace {
    fn from_raw(value: u32) -> Self {
        match value {
            16 => EnumeratedColourspace::Srgb,
            17 => EnumeratedColourspace::Greyscale,
            18 => EnumeratedColourspace::Sycc,
            v => EnumeratedColourspace::Reserved(v),
        }
    }
}

/// Association between codestream components/channels and their semantic role.
#[derive(Debug, Clone)]
pub struct ChannelDefinition {
    pub channel_index: u16,
    pub channel_type: ChannelType,
    pub association: ChannelAssociation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    Colour,
    Opacity,
    PremultipliedOpacity,
    Reserved(u16),
    Unspecified,
}

impl ChannelType {
    fn from_raw(value: u16) -> Self {
        match value {
            0 => ChannelType::Colour,
            1 => ChannelType::Opacity,
            2 => ChannelType::PremultipliedOpacity,
            u16::MAX => ChannelType::Unspecified,
            v => ChannelType::Reserved(v),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAssociation {
    WholeImage,
    Colour(u16),
    Unspecified,
}

impl ChannelAssociation {
    fn from_raw(value: u16) -> Self {
        match value {
            0 => ChannelAssociation::WholeImage,
            u16::MAX => ChannelAssociation::Unspecified,
            v => ChannelAssociation::Colour(v),
        }
    }
}

impl ImageMetadata {
    /// Parse Image Header box (ihdr) data.
    fn parse_ihdr(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() < 14 {
            return Err("image header box too short");
        }

        let mut reader = Reader::new(data);

        self.height = reader
            .read_u32()
            .ok_or("failed to read image height from header")?;
        self.width = reader
            .read_u32()
            .ok_or("failed to read image width from header")?;
        let _num_components = reader
            .read_u16()
            .ok_or("failed to read component count from header")?;
        let bits_per_component = reader
            .read_byte()
            .ok_or("failed to read bits per component from header")?;

        if bits_per_component == 255 {
            return Err("extended bits-per-component header unsupported");
        }

        let _compression_type = reader
            .read_byte()
            .ok_or("failed to read compression type from header")?;
        let _colorspace_unknown = reader
            .read_byte()
            .ok_or("failed to read colourspace flag from header")?;
        let _has_intellectual_property = reader
            .read_byte()
            .ok_or("failed to read intellectual property flag from header")?;

        Ok(())
    }

    /// Parse Channel Definition box (cdef) data.
    fn parse_cdef(&mut self, data: &[u8]) -> Option<()> {
        if data.len() < 2 {
            return None;
        }

        let mut reader = Reader::new(data);
        let count = reader.read_u16()? as usize;
        let mut definitions = Vec::with_capacity(count);

        for _ in 0..count {
            let channel_index = reader.read_u16()?;
            let channel_type = reader.read_u16()?;
            let association = reader.read_u16()?;

            definitions.push(ChannelDefinition {
                channel_index,
                channel_type: ChannelType::from_raw(channel_type),
                association: ChannelAssociation::from_raw(association),
            });
        }

        self.channel_definitions = definitions;
        Some(())
    }

    /// Parse Colour Specification box (colr) data.
    fn parse_colr(&mut self, data: &[u8]) -> Option<()> {
        if data.len() < 3 {
            return None;
        }

        let mut reader = Reader::new(data);

        let meth = reader.read_byte()?;
        let prec = reader.read_byte()?;
        let approx = reader.read_byte()?;

        let method = match meth {
            1 => {
                let enumerated = reader.read_u32()?;
                ColourSpecificationMethod::Enumerated(EnumeratedColourspace::from_raw(enumerated))
            }
            2 => {
                let profile_data = reader.tail()?.to_vec();
                ColourSpecificationMethod::IccProfile(profile_data)
            }
            v => ColourSpecificationMethod::Unknown(v),
        };

        self.colour_specification = Some(ColourSpecification {
            method,
            precedence: prec,
            approximation: approx,
        });

        Some(())
    }
}

pub fn read(data: &[u8]) -> Result<Bitmap, &'static str> {
    // JP2 signature box: 00 00 00 0C 6A 50 20 20
    const JP2_MAGIC: &[u8] = b"\x00\x00\x00\x0C\x6A\x50\x20\x20";
    // Codestream signature: FF 4F FF 51 (SOC + SIZ markers)
    const CODESTREAM_MAGIC: &[u8] = b"\xFF\x4F\xFF\x51";
    if data.starts_with(JP2_MAGIC) {
        read_jp2_file(data)
    } else if data.starts_with(CODESTREAM_MAGIC) {
        read_jp2_codestream(data)
    } else {
        Err("invalid JP2 file")
    }
}

fn read_jp2_codestream(data: &[u8]) -> Result<Bitmap, &'static str> {
    let (header, channels) = codestream::read(data)?;

    let metadata = ImageMetadata {
        height: header.size_data.image_height(),
        width: header.size_data.image_width(),
        has_intellectual_property: 0,
        colour_specification: None,
        channel_definitions: vec![],
    };

    Ok(Bitmap { channels, metadata })
}

fn read_jp2_file(data: &[u8]) -> Result<Bitmap, &'static str> {
    let mut reader = Reader::new(data);
    let signature_box = read_box(&mut reader).ok_or("failed to read signature box")?;

    if signature_box.box_type != JP2_SIGNATURE {
        return Err("invalid JP2 signature");
    }

    let file_type_box = read_box(&mut reader).ok_or("failed to read file type box")?;

    if file_type_box.box_type != FILE_TYPE {
        return Err("invalid JP2 file type");
    }

    let mut metadata = Err("failed to read metadata");
    let mut channels = Err("failed to decode image");

    // Read boxes until we find the JP2 Header box
    while !reader.at_end() {
        let Some(current_box) = read_box(&mut reader) else {
            warn!("failed to read a JP2 box, aborting");

            break;
        };

        if current_box.box_type == JP2_HEADER {
            // Parse the JP2 Header box (superbox)
            let mut image_metadata = ImageMetadata {
                height: 0,
                width: 0,
                has_intellectual_property: 0,
                colour_specification: None,
                channel_definitions: Vec::new(),
            };

            let mut jp2h_reader = Reader::new(current_box.data);

            // Read child boxes within JP2 Header box
            while !jp2h_reader.at_end() {
                let child_box = read_box(&mut jp2h_reader).ok_or("failed to read JP2 box")?;

                match child_box.box_type {
                    IMAGE_HEADER => {
                        image_metadata.parse_ihdr(child_box.data)?;
                    }
                    CHANNEL_DEFINITION => {
                        image_metadata
                            .parse_cdef(child_box.data)
                            .ok_or("failed to parse channel definition")?;
                    }
                    COLOUR_SPECIFICATION => {
                        image_metadata
                            .parse_colr(child_box.data)
                            .ok_or("failed to parse colour")?;
                    }
                    _ => {
                        debug!("ignoring box {}", tag_to_string(child_box.box_type));
                    }
                }
            }

            metadata = Ok(image_metadata);
        } else if current_box.box_type == CONTIGUOUS_CODESTREAM {
            channels = Ok(codestream::read(current_box.data)?);
        } else {
            debug!("ignoring outer box {}", tag_to_string(current_box.box_type));
        }
    }

    let (_, mut channels) = channels?;
    let metadata = metadata?;

    for (idx, channel) in channels.iter_mut().enumerate() {
        channel.is_alpha = metadata
            .channel_definitions
            .get(idx)
            .map(|c| c.channel_type == ChannelType::Opacity)
            .unwrap_or(false);
    }

    Ok(Bitmap { channels, metadata })
}
