#![forbid(unsafe_code)]

use crate::j2c::ComponentData;
use crate::jp2::{DecodedImage, ImageBoxes};
use crate::jp2::cdef::{ChannelAssociation, ChannelType};
use crate::jp2::cmap::ComponentMappingType;
use crate::jp2::colr::EnumeratedColorspace;
use crate::jp2::icc::ICCMetadata;

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
        num_components: u8,
    },
}

impl ColorSpace {
    pub fn num_components(&self) -> u8 {
        match self {
            ColorSpace::Gray => 1,
            ColorSpace::RGB => 3,
            ColorSpace::CMYK => 4,
            ColorSpace::Icc { num_components, .. } => *num_components
        }
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

    // Resolve palette indices.
    if settings.resolve_palette_indices {
        decoded_image.decoded.components =
            resolve_palette_indices(decoded_image.decoded.components, &decoded_image.boxes)
                .ok_or("failed to resolve palette indices")?;
    }

    // Check that we only have at most one alpha channel, and that the alpha
    // chanel is the last component.
    let mut has_alpha = false;
    
    let bit_depth = decoded_image.decoded.components[0].bit_depth;
    
    // Validate that all channels have the same bit-depth.
    for component in &decoded_image.decoded.components {
        if component.bit_depth != bit_depth {
            return Err("images with varying bit depths per channel are not supported.");
        }
    }

    if let Some(cdef) = &decoded_image.boxes.channel_definition {
        // Note that in the `parse` method we validated that there is at least
        // one definition.
        for def in &cdef.channel_definitions[..cdef.channel_definitions.len() - 1] {
            if def.channel_type == ChannelType::Opacity
                || def.association == ChannelAssociation::WholeImage
            {
                return Err("unsupported JP image");
            }
        }

        let last = cdef.channel_definitions.last().unwrap();

        if (last.channel_type == ChannelType::Colour
            && matches!(last.association, ChannelAssociation::Colour(_)))
            || (last.channel_type == ChannelType::Opacity
                && matches!(last.association, ChannelAssociation::Colour(_)))
        {
            return Err("unsupported JP image");
        }

        has_alpha = last.channel_type == ChannelType::Opacity;
    }
    
    let mut color_space = resolve_color_space(&mut decoded_image, bit_depth)?;
    
    // If we didn't resolve palette indices, we need to assume grayscale image.
    if !settings.resolve_palette_indices {
        has_alpha = false;
        color_space = ColorSpace::Gray;
    }
    
    // Validate the number of channels.
    if decoded_image.decoded.components.len() != (color_space.num_components() + if has_alpha { 1} else { 0 }) as usize {
        return Err("unsupported JP image");
    }

    unimplemented!()
}

fn resolve_color_space(
    image: &mut DecodedImage,
    bit_depth: u8,
) -> Result<ColorSpace, &'static str> {
    let cs = match &image.boxes.color_specification.as_ref().unwrap().color_space {
        jp2::colr::ColorSpace::Enumerated(e) => {
            match e {
                EnumeratedColorspace::Cmyk => ColorSpace::CMYK,
                EnumeratedColorspace::Srgb => ColorSpace::RGB,
                EnumeratedColorspace::RommRgb => {
                    // Use an ICC profile to process the RommRGB color space.
                    ColorSpace::Icc {
                        profile: include_bytes!("../assets/ISO22028-2_ROMM-RGB.icc").to_vec(),
                        num_components: 3,
                    }
                }
                EnumeratedColorspace::Greyscale => ColorSpace::Gray,
                EnumeratedColorspace::Sycc => {
                    sycc_to_rgb(
                        &mut image.decoded.components, bit_depth
                    )
                        .ok_or("failed to convert image from sycc to RGB")?;

                    ColorSpace::RGB
                }
                _ => return Err("unsupported JP2 image"),
            }
        }
        jp2::colr::ColorSpace::Icc(icc) => {
            let metadata = ICCMetadata::from_data(&icc)
                .ok_or("invalid ICC metadata")?;

            ColorSpace::Icc {
                profile: icc.clone(),
                num_components: metadata.color_space.num_components(),
            }
        }
    };
    
    Ok(cs)
}

fn resolve_palette_indices(
    components: Vec<ComponentData>,
    boxes: &ImageBoxes,
) -> Option<Vec<ComponentData>> {
    let Some(palette) = boxes.palette.as_ref() else {
        // Nothing to resolve.
        return Some(components);
    };

    let mapping = boxes.component_mapping.as_ref().unwrap();
    let mut resolved = Vec::with_capacity(mapping.entries.len());

    for entry in &mapping.entries {
        let component_idx = entry.component_index as usize;
        let component = components.get(component_idx)?;

        match entry.mapping_type {
            ComponentMappingType::Direct => resolved.push(component.clone()),
            ComponentMappingType::Palette { column } => {
                let column_idx = column as usize;
                let column_info = palette.columns.get(column_idx)?;

                let mut mapped = Vec::with_capacity(component.container.len());

                for &sample in &component.container {
                    let index = sample.round() as i64;
                    let value = palette.map(index as usize, column_idx)?;
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

fn sycc_to_rgb(components: &mut [ComponentData], bit_depth: u8) -> Option<()> {
    let offset = (1u32 << (bit_depth as u32 - 1)) as f32;
    let max_value = ((1u32 << bit_depth as u32) - 1) as f32;
    
    let (head, _) = components.split_at_mut_checked(3)?;

    let [y, cb, cr] = head else {
        unreachable!();
    };

    for ((y, cb), cr) in y
        .container
        .iter_mut()
        .zip(cb.container.iter_mut())
        .zip(cr.container.iter_mut())
    {
        *cb -= offset;
        *cr -= offset;

        let r = *y + 1.402_f32 * *cr;
        let g = *y - 0.344136_f32 * *cb - 0.714136_f32 * *cr;
        let b = *y + 1.772_f32 * *cb;

        // min + max is better than clamp in terms of performance.
        *y = r.min(max_value).max(0.0);
        *cb = g.min(max_value).max(0.0);
        *cr = b.min(max_value).max(0.0);
    }
    
    Some(())
}
