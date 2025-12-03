use crate::jp2::cdef::{ChannelDefinition, ChannelDefinitionBox};
use crate::jp2::colr::ColorSpecificationBox;
use crate::reader::BitReader;

mod r#box;
mod colr;
mod icc;
mod cdef;

#[derive(Debug, Clone)]
pub(crate) struct ImageBoxes {
    pub(crate) color_specification: Option<ColorSpecificationBox>,
    pub(crate) channel_definition: Option<ChannelDefinitionBox>,
    /// Palette definitions from the Palette box (pclr).
    pub(crate) palette: Option<Palette>,
    /// Component mappings defined by the Component Mapping box (cmap).
    pub(crate) component_mapping: Option<Vec<ComponentMappingEntry>>,
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

impl ImageBoxes {
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