#![forbid(unsafe_code)]

use crate::j2c::ComponentData;
use crate::jp2::cmap::ComponentMappingType;
use crate::jp2::ImageBoxes;

mod j2c;
mod jp2;
pub(crate) mod reader;

#[derive(Debug, Copy, Clone)]
pub struct DecodeSettings {
    /// Whether palette indices should be resolved.
    pub resolve_palette_indices: bool,
    /// Whether strict mode should be enabled when decoding.
    ///
    /// It is recommended to leave this flag disabled, unless you have a
    /// specific reason not to.
    pub strict: bool,
}

impl Default for DecodeSettings {
    fn default() -> Self {
        Self {
            resolve_palette_indices: true,
            strict: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ColorSpace {
    Gray,
    RGB,
    CMYK,
    Icc {
        profile: Vec<u8>,
        num_components: usize,
    }
}

pub fn read(data: &[u8], settings: &DecodeSettings) -> Result<(), &'static str> {
    // JP2 signature box: 00 00 00 0C 6A 50 20 20
    const JP2_MAGIC: &[u8] = b"\x00\x00\x00\x0C\x6A\x50\x20\x20";
    // Codestream signature: FF 4F FF 51 (SOC + SIZ markers)
    const CODESTREAM_MAGIC: &[u8] = b"\xFF\x4F\xFF\x51";

    let mut decoded_image = if data.starts_with(JP2_MAGIC) {
        jp2::decode(data, settings)?
    } else if data.starts_with(CODESTREAM_MAGIC) {
        j2c::decode(data, settings)?
    } else {
        return Err("invalid JP2 file");
    };
    
    if settings.resolve_palette_indices {
        decoded_image.decoded.components = resolve_palette_indices(decoded_image.decoded.components, &decoded_image.boxes)
            .ok_or("failed to resolve palette indices")?;
    }
    
    
}

fn resolve_palette_indices(
    components: Vec<ComponentData>,
    boxes: &ImageBoxes,
) -> Option<Vec<ComponentData>> {
    let Some(palette) = boxes.palette.as_ref() else {
        // Nothing to resolve.
        return Some(components)
    };

    let mapping = boxes.component_mapping.as_ref().unwrap();
    let mut resolved = Vec::with_capacity(mapping.entries.len());

    for entry in &mapping.entries {
        let component_idx = entry.component_index as usize;
        let component = components
            .get(component_idx)?;

        match entry.mapping_type {
            ComponentMappingType::Direct => resolved.push(component.clone()),
            ComponentMappingType::Palette { column } => {
                let column_idx = column as usize;
                let column_info = palette
                    .columns
                    .get(column_idx)?;

                let mut mapped = Vec::with_capacity(component.container.len());

                for &sample in &component.container {
                    let index = sample.round() as i64;
                    let value = palette
                        .map(index as usize, column_idx)?;
                    mapped.push(value as f32);
                }

                resolved.push(ComponentData {
                    container: mapped,
                    bit_depth: column_info.bit_depth,
                });
            }
        }
    }

    Some(resolved)
}
