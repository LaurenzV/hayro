#![forbid(unsafe_code)]

use crate::bitmap::{Bitmap, ChannelData};
use crate::boxes::{
    CHANNEL_DEFINITION, COLOUR_SPECIFICATION, COMPONENT_MAPPING, CONTIGUOUS_CODESTREAM, FILE_TYPE,
    IMAGE_HEADER, JP2_HEADER, JP2_SIGNATURE, PALETTE, read_box, tag_to_string,
};
use crate::byte_reader::Reader;
use crate::icc::ICCMetadata;
use log::{debug, warn};

mod arithmetic_decoder;
pub(crate) mod bit_reader;
pub mod bitmap;
pub(crate) mod bitplane;
pub mod boxes;
pub(crate) mod byte_reader;
mod codestream;
mod decode;
mod icc;
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
    /// Palette definitions from the Palette box (pclr).
    pub palette: Option<Palette>,
    /// Component mappings defined by the Component Mapping box (cmap).
    pub component_mapping: Option<Vec<ComponentMappingEntry>>,
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

impl ColourSpecificationMethod {
    pub fn expected_number_of_channels(&self) -> Option<u8> {
        match self {
            ColourSpecificationMethod::Enumerated(e) => e.expected_number_of_channels(),
            ColourSpecificationMethod::IccProfile(i) => {
                Some(
                    ICCMetadata::from_data(i)
                        .map(|d| d.color_space.num_components())
                        // Let's just assume RGB.
                        .unwrap_or(3),
                )
            }
            ColourSpecificationMethod::Unknown(_) => None,
        }
    }
}

/// Enumerated colourspace identifiers defined by ISO/IEC 15444-1 Table L.10
/// and Table M.25.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumeratedColourspace {
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
    Reserved(u32),
}

impl EnumeratedColourspace {
    fn from_raw(value: u32) -> Self {
        match value {
            0 => EnumeratedColourspace::BiLevel1,
            1 => EnumeratedColourspace::YCbCr1,
            3 => EnumeratedColourspace::YCbCr2,
            4 => EnumeratedColourspace::YCbCr3,
            9 => EnumeratedColourspace::PhotoYcc,
            11 => EnumeratedColourspace::Cmy,
            12 => EnumeratedColourspace::Cmyk,
            13 => EnumeratedColourspace::Ycck,
            14 => EnumeratedColourspace::CieLab,
            15 => EnumeratedColourspace::BiLevel2,
            16 => EnumeratedColourspace::Srgb,
            17 => EnumeratedColourspace::Greyscale,
            18 => EnumeratedColourspace::Sycc,
            19 => EnumeratedColourspace::CieJab,
            20 => EnumeratedColourspace::EsRgb,
            21 => EnumeratedColourspace::RommRgb,
            22 => EnumeratedColourspace::YPbPr112560,
            23 => EnumeratedColourspace::YPbPr125050,
            24 => EnumeratedColourspace::EsYcc,
            25 => EnumeratedColourspace::ScRgb,
            26 => EnumeratedColourspace::ScRgbGray,
            v => EnumeratedColourspace::Reserved(v),
        }
    }

    /// Returns the number of colour channels this enumerated space expects without accounting
    /// for extra alpha channels.
    pub fn expected_number_of_channels(&self) -> Option<u8> {
        match self {
            EnumeratedColourspace::BiLevel1 => Some(1),
            EnumeratedColourspace::YCbCr1 => Some(3),
            EnumeratedColourspace::YCbCr2 => Some(3),
            EnumeratedColourspace::YCbCr3 => Some(3),
            EnumeratedColourspace::PhotoYcc => Some(3),
            EnumeratedColourspace::Cmy => Some(3),
            EnumeratedColourspace::Cmyk => Some(4),
            EnumeratedColourspace::Ycck => Some(4),
            EnumeratedColourspace::CieLab => Some(3),
            EnumeratedColourspace::BiLevel2 => Some(1),
            EnumeratedColourspace::Srgb => Some(3),
            EnumeratedColourspace::Greyscale => Some(1),
            EnumeratedColourspace::Sycc => Some(3),
            EnumeratedColourspace::CieJab => Some(3),
            EnumeratedColourspace::EsRgb => Some(3),
            EnumeratedColourspace::RommRgb => Some(3),
            EnumeratedColourspace::YPbPr112560 => Some(3),
            EnumeratedColourspace::YPbPr125050 => Some(3),
            EnumeratedColourspace::EsYcc => Some(3),
            EnumeratedColourspace::ScRgb => Some(3),
            EnumeratedColourspace::ScRgbGray => Some(1),
            EnumeratedColourspace::Reserved(_) => None,
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

#[derive(Debug, Clone)]
pub struct Palette {
    pub entries: Vec<Vec<i64>>,
    pub columns: Vec<PaletteColumn>,
}

impl Palette {
    fn value(&self, entry: usize, column: usize) -> Option<i64> {
        self.entries
            .get(entry)
            .and_then(|row| row.get(column))
            .copied()
    }

    fn num_entries(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PaletteColumn {
    pub bit_depth: u8,
    pub is_signed: bool,
}

#[derive(Debug, Clone)]
pub struct ComponentMappingEntry {
    pub component_index: u16,
    pub mapping_type: ComponentMappingType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentMappingType {
    Direct,
    Palette { column: u8 },
    Reserved(u8),
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

    /// Parse Palette box (pclr) data.
    fn parse_pclr(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() < 3 {
            return Err("palette box too short");
        }

        let mut reader = Reader::new(data);
        let num_entries = reader
            .read_u16()
            .ok_or("failed to read palette entry count")? as usize;
        let num_components = reader
            .read_byte()
            .ok_or("failed to read palette component count")? as usize;

        if num_entries == 0 || num_components == 0 {
            return Err("palette must contain entries and components");
        }

        let mut columns = Vec::with_capacity(num_components);
        for _ in 0..num_components {
            let descriptor = reader
                .read_byte()
                .ok_or("failed to read palette column descriptor")?;
            let bit_depth = (descriptor & 0x7F)
                .checked_add(1)
                .ok_or("invalid palette bit depth")?;
            columns.push(PaletteColumn {
                bit_depth,
                is_signed: (descriptor & 0x80) != 0,
            });
        }

        let mut entries = Vec::with_capacity(num_entries);
        for _ in 0..num_entries {
            let mut row = Vec::with_capacity(num_components);
            for column in &columns {
                let num_bytes = (column.bit_depth as usize).div_ceil(8).max(1);
                let raw_bytes = reader
                    .read_bytes(num_bytes)
                    .ok_or("failed to read palette entry values")?;
                let mut raw_value = 0u64;
                for &byte in raw_bytes {
                    raw_value = (raw_value << 8) | byte as u64;
                }

                let value = if column.is_signed {
                    let shift = 64 - column.bit_depth as u32;
                    (raw_value << shift) as i64 >> shift
                } else {
                    raw_value as i64
                };

                row.push(value);
            }

            entries.push(row);
        }

        self.palette = Some(Palette { entries, columns });
        Ok(())
    }

    /// Parse Component Mapping box (cmap) data.
    fn parse_cmap(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if !data.len().is_multiple_of(4) {
            return Err("component mapping box has invalid length");
        }

        let mut reader = Reader::new(data);
        let mut entries = Vec::with_capacity(data.len() / 4);

        while !reader.at_end() {
            let component_index = reader
                .read_u16()
                .ok_or("failed to read component index from cmap box")?;
            let mapping_type = reader
                .read_byte()
                .ok_or("failed to read mapping type from cmap box")?;
            let palette_column = reader
                .read_byte()
                .ok_or("failed to read palette column from cmap box")?;

            let mapping_type = match mapping_type {
                0 => ComponentMappingType::Direct,
                1 => ComponentMappingType::Palette {
                    column: palette_column,
                },
                other => ComponentMappingType::Reserved(other),
            };

            entries.push(ComponentMappingEntry {
                component_index,
                mapping_type,
            });
        }

        self.component_mapping = Some(entries);
        Ok(())
    }
}

fn resolve_component_channels(
    channels: Vec<ChannelData>,
    metadata: &ImageMetadata,
) -> Result<Vec<ChannelData>, &'static str> {
    let mapping = if let Some(mapping) = metadata.component_mapping.clone() {
        mapping
    } else if let Some(palette) = metadata.palette.as_ref() {
        // In theory, a cmap is required if we have pclr, but we intead assume
        // that all channels are mapped via the palette in case not.
        (0..palette.columns.len())
            .map(|i| ComponentMappingEntry {
                component_index: 0,
                mapping_type: ComponentMappingType::Palette { column: i as u8 },
            })
            .collect::<Vec<_>>()
    } else {
        return Ok(channels);
    };

    let mut resolved = Vec::with_capacity(mapping.len());

    for entry in mapping {
        let component_idx = entry.component_index as usize;
        let component = channels
            .get(component_idx)
            .ok_or("component mapping references invalid component")?;

        match entry.mapping_type {
            ComponentMappingType::Direct => resolved.push(component.clone()),
            ComponentMappingType::Palette { column } => {
                let palette = metadata
                    .palette
                    .as_ref()
                    .ok_or("component mapping requires palette box")?;
                let column_idx = column as usize;
                let column_info = palette
                    .columns
                    .get(column_idx)
                    .ok_or("component mapping references missing palette column")?;

                let mut mapped = Vec::with_capacity(component.container.len());
                for &sample in &component.container {
                    let index = sample.round() as i64;
                    if index < 0 || (index as usize) >= palette.num_entries() {
                        return Err("palette index out of range");
                    }

                    let value = palette
                        .value(index as usize, column_idx)
                        .ok_or("palette entry missing value")?;
                    mapped.push(value as f32);
                }

                resolved.push(ChannelData {
                    container: mapped,
                    bit_depth: column_info.bit_depth,
                    is_alpha: false,
                });
            }
            ComponentMappingType::Reserved(_) => {
                return Err("unsupported component mapping type");
            }
        }
    }

    Ok(resolved)
}

#[derive(Debug, Copy, Clone)]
pub struct DecodeSettings {
    /// Whether palette indices should be resolved.
    pub resolve_palette_indices: bool,
}

impl Default for DecodeSettings {
    fn default() -> Self {
        Self {
            resolve_palette_indices: true,
        }
    }
}

pub fn read(data: &[u8], settings: &DecodeSettings) -> Result<Bitmap, &'static str> {
    // JP2 signature box: 00 00 00 0C 6A 50 20 20
    const JP2_MAGIC: &[u8] = b"\x00\x00\x00\x0C\x6A\x50\x20\x20";
    // Codestream signature: FF 4F FF 51 (SOC + SIZ markers)
    const CODESTREAM_MAGIC: &[u8] = b"\xFF\x4F\xFF\x51";
    if data.starts_with(JP2_MAGIC) {
        read_jp2_file(data, settings)
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
        colour_specification: {
            let method = if channels.len() < 3 {
                EnumeratedColourspace::Greyscale
            } else {
                EnumeratedColourspace::Srgb
            };

            Some(ColourSpecification {
                method: ColourSpecificationMethod::Enumerated(method),
                precedence: 0,
                approximation: 0,
            })
        },
        channel_definitions: vec![],
        palette: None,
        component_mapping: None,
    };

    Ok(Bitmap { channels, metadata })
}

fn read_jp2_file(data: &[u8], settings: &DecodeSettings) -> Result<Bitmap, &'static str> {
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

        match current_box.box_type {
            JP2_HEADER => {
                // Parse the JP2 Header box (superbox)
                let mut image_metadata = ImageMetadata {
                    height: 0,
                    width: 0,
                    has_intellectual_property: 0,
                    colour_specification: None,
                    channel_definitions: Vec::new(),
                    palette: None,
                    component_mapping: None,
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
                        PALETTE => {
                            image_metadata
                                .parse_pclr(child_box.data)
                                .map_err(|_| "failed to parse palette")?;
                        }
                        COMPONENT_MAPPING => {
                            image_metadata
                                .parse_cmap(child_box.data)
                                .map_err(|_| "failed to parse component mapping")?;
                        }
                        _ => {
                            debug!("ignoring box {}", tag_to_string(child_box.box_type));
                        }
                    }
                }

                if image_metadata.width == 0 || image_metadata.height == 0 {
                    return Err("image has invalid dimensions");
                }

                metadata = Ok(image_metadata);
            }
            CONTIGUOUS_CODESTREAM => {
                channels = Ok(codestream::read(current_box.data)?);
            }
            _ => {
                warn!("ignoring outer box {}", tag_to_string(current_box.box_type));
            }
        }
    }

    let (header, mut channels) = channels?;
    let mut metadata = metadata?;

    // In case header and codestream have inconsistent size metadata, use the
    // one from the codestream.
    metadata.width = header.size_data.image_width();
    metadata.height = header.size_data.image_height();

    if settings.resolve_palette_indices {
        channels = resolve_component_channels(channels, &metadata)?;
    }

    for (idx, channel) in channels.iter_mut().enumerate() {
        channel.is_alpha = metadata
            .channel_definitions
            .get(idx)
            .map(|c| c.channel_type == ChannelType::Opacity)
            .unwrap_or(false);
    }

    Ok(Bitmap { channels, metadata })
}
