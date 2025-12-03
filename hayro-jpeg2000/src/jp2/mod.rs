use crate::reader::BitReader;

mod r#box;
mod colr;
mod icc;

/// Image metadata extracted from JP2 Header box.
#[derive(Debug, Clone)]
pub struct ImageMetadata {
    /// Image area height in reference grid points.
    pub height: u32,
    /// Image area width in reference grid points.
    pub width: u32,
    /// Colour specification information from the Colour Specification box.
    pub colour_specification: Option<ColourSpecification>,
    /// Channel definitions specified by the Channel Definition box (cdef).
    pub channel_definitions: Vec<ChannelDefinition>,
    /// Palette definitions from the Palette box (pclr).
    pub palette: Option<Palette>,
    /// Component mappings defined by the Component Mapping box (cmap).
    pub component_mapping: Option<Vec<ComponentMappingEntry>>,
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

        let mut reader = BitReader::new(data);

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

        let mut reader = BitReader::new(data);
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

    

    /// Parse Palette box (pclr) data.
    fn parse_pclr(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() < 3 {
            return Err("palette box too short");
        }

        let mut reader = BitReader::new(data);
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

        let mut reader = BitReader::new(data);
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