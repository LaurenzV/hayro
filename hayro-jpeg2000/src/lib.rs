#![forbid(unsafe_code)]

mod j2c;
mod jp2;
pub(crate) mod reader;

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

pub fn read(data: &[u8], settings: &DecodeSettings) -> Result<Bitmap, &'static str> {
    // JP2 signature box: 00 00 00 0C 6A 50 20 20
    const JP2_MAGIC: &[u8] = b"\x00\x00\x00\x0C\x6A\x50\x20\x20";
    // Codestream signature: FF 4F FF 51 (SOC + SIZ markers)
    const CODESTREAM_MAGIC: &[u8] = b"\xFF\x4F\xFF\x51";

    let decoded_image = if data.starts_with(JP2_MAGIC) {
        jp2::decode(data, settings)
    } else if data.starts_with(CODESTREAM_MAGIC) {
        j2c::decode(data, settings)
    } else {
        Err("invalid JP2 file")
    }?;
}
