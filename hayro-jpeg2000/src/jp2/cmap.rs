//! The component mapping box (cmap), defined in I.5.3.5.

use crate::jp2::ImageBoxes;
use crate::reader::BitReader;

pub(crate) fn parse(boxes: &mut ImageBoxes, data: &[u8]) -> Option<()> {
    let mut reader = BitReader::new(data);
    let mut entries = Vec::with_capacity(data.len() / 4);

    while !reader.at_end() {
        let component_index = reader.read_u16()?;
        let mapping_type = reader.read_byte()?;
        let palette_column = reader.read_byte()?;

        let mapping_type = match mapping_type {
            0 => ComponentMappingType::Direct,
            1 => ComponentMappingType::Palette {
                column: palette_column,
            },
            _ => return None,
        };

        entries.push(ComponentMappingEntry {
            component_index,
            mapping_type,
        });
    }

    boxes.component_mapping = Some(ComponentMappingBox { entries });

    Some(())
}

#[derive(Debug, Clone)]
pub(crate) struct ComponentMappingBox {
    pub(crate) entries: Vec<ComponentMappingEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct ComponentMappingEntry {
    pub(crate) component_index: u16,
    pub(crate) mapping_type: ComponentMappingType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentMappingType {
    Direct,
    Palette { column: u8 },
}
