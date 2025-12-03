#![forbid(unsafe_code)]

use crate::bitmap::{Bitmap, ChannelData};
use crate::boxes::{
    CHANNEL_DEFINITION, COLOUR_SPECIFICATION, COMPONENT_MAPPING, CONTIGUOUS_CODESTREAM, FILE_TYPE,
    IMAGE_HEADER, JP2_HEADER, JP2_SIGNATURE, PALETTE, read_box, tag_to_string,
};
use crate::reader::BitReader;
use log::{debug, warn};

mod arithmetic_decoder;
pub mod bitmap;
pub(crate) mod bitplane;
mod build;
mod codestream;
mod decode;
pub(crate) mod idwt;
mod jp2;
mod mct;
mod progression;
pub(crate) mod reader;
pub(crate) mod rect;
mod segment;
mod tag_tree;
mod tile;

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
    if data.starts_with(JP2_MAGIC) {
        read_jp2_file(data, settings)
    } else if data.starts_with(CODESTREAM_MAGIC) {
        read_jp2_codestream(data, settings)
    } else {
        Err("invalid JP2 file")
    }
}

fn read_jp2_codestream(data: &[u8], settings: &DecodeSettings) -> Result<Bitmap, &'static str> {
    let (header, channels) = codestream::read(data, settings)?;

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
    let mut reader = BitReader::new(data);
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
            if settings.strict {
                return Err("failed to read a JP2 box");
            }

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

                let mut jp2h_reader = BitReader::new(current_box.data);

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
                channels = Ok(codestream::read(current_box.data, settings)?);
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
