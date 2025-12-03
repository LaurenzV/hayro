#![forbid(unsafe_code)]

use crate::j2c::ComponentData;
use crate::jp2::cdef::{ChannelAssociation, ChannelType};
use crate::jp2::cmap::ComponentMappingType;
use crate::jp2::colr::EnumeratedColorspace;
use crate::jp2::icc::ICCMetadata;
use crate::jp2::{DecodedImage, ImageBoxes};

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
    pub fn num_channels(&self) -> u8 {
        match self {
            ColorSpace::Gray => 1,
            ColorSpace::RGB => 3,
            ColorSpace::CMYK => 4,
            ColorSpace::Icc { num_components, .. } => *num_components,
        }
    }
}

pub struct Bitmap {
    pub color_space: ColorSpace,
    pub data: Vec<u8>,
    pub has_alpha: bool,
    pub width: u32,
    pub height: u32,
    pub original_bit_depth: u8,
}

pub fn read(data: &[u8], settings: &DecodeSettings) -> Result<Bitmap, &'static str> {
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

    let width = decoded_image.decoded.width;
    let height = decoded_image.decoded.height;

    // Resolve palette indices.
    if settings.resolve_palette_indices {
        decoded_image.decoded.components =
            resolve_palette_indices(decoded_image.decoded.components, &decoded_image.boxes)
                .ok_or("failed to resolve palette indices")?;
    }

    // Check that we only have at most one alpha channel, and that the alpha
    // chanel is the last component.
    let mut has_alpha = false;

    if let Some(cdef) = &decoded_image.boxes.channel_definition {
        let last = cdef.channel_definitions.last().unwrap();
        has_alpha = last.channel_type == ChannelType::Opacity;

        // Sort by the channel association. Note that this will only work if
        // each component is referenced only once.
        let mut components = decoded_image
            .decoded
            .components
            .into_iter()
            .zip(
                cdef.channel_definitions
                    .iter()
                    .map(|c| match c._association {
                        ChannelAssociation::WholeImage => u16::MAX,
                        ChannelAssociation::Colour(c) => c,
                    }),
            )
            .collect::<Vec<_>>();
        components.sort_by(|c1, c2| c1.1.cmp(&c2.1));
        decoded_image.decoded.components = components.into_iter().map(|c| c.0).collect();
    }

    // Note that this is only valid if all images have the same bit depth.
    let bit_depth = decoded_image.decoded.components[0].bit_depth;

    let mut color_space = resolve_color_space(&mut decoded_image, bit_depth)?;

    // If we didn't resolve palette indices, we need to assume grayscale image.
    if !settings.resolve_palette_indices && decoded_image.boxes.palette.is_some() {
        has_alpha = false;
        color_space = ColorSpace::Gray;
    }

    // Validate the number of channels.
    if decoded_image.decoded.components.len()
        != (color_space.num_channels() + if has_alpha { 1 } else { 0 }) as usize
    {
        if !settings.strict
            && decoded_image.decoded.components.len() == color_space.num_channels() as usize + 1
            && !has_alpha
        {
            // See OPENJPEG test case orb-blue10-lin-j2k. Assume that we have an
            // alpha channel in this case.
            has_alpha = true;
        } else {
            return Err("image has too many channels");
        }
    }

    Ok(Bitmap {
        color_space,
        has_alpha,
        original_bit_depth: bit_depth,
        data: interleave_and_convert(decoded_image),
        width,
        height,
    })
}

fn interleave_and_convert(image: DecodedImage) -> Vec<u8> {
    let mut components = image.decoded.components;
    let num_components = components.len();

    let mut all_same_bit_depth = Some(components[0].bit_depth);

    for component in components.iter().skip(1) {
        if Some(component.bit_depth) != all_same_bit_depth {
            all_same_bit_depth = None;
        }
    }

    let max_len = components[0].container.len();

    if all_same_bit_depth == Some(8) && num_components <= 4 {
        // Fast path for the common case.
        match num_components {
            // Gray-scale.
            1 => components[0]
                .container
                .iter()
                .map(|v| v.round() as u8)
                .collect(),
            // Gray-scale with alpha.
            2 => {
                let c1 = components.pop().unwrap();
                let c0 = components.pop().unwrap();

                let c0 = &c0.container[..max_len];
                let c1 = &c1.container[..max_len];

                let mut data = Vec::with_capacity(max_len * 2);

                for i in 0..max_len {
                    data.push(c0[i].round() as u8);
                    data.push(c1[i].round() as u8);
                }

                data
            }
            // RGB
            3 => {
                let c2 = components.pop().unwrap();
                let c1 = components.pop().unwrap();
                let c0 = components.pop().unwrap();

                let c0 = &c0.container[..max_len];
                let c1 = &c1.container[..max_len];
                let c2 = &c2.container[..max_len];

                let mut data = Vec::with_capacity(max_len * 3);

                for i in 0..max_len {
                    data.push(c0[i].round() as u8);
                    data.push(c1[i].round() as u8);
                    data.push(c2[i].round() as u8);
                }

                data
            }
            // RGBA or CMYK.
            4 => {
                let c3 = components.pop().unwrap();
                let c2 = components.pop().unwrap();
                let c1 = components.pop().unwrap();
                let c0 = components.pop().unwrap();

                let c0 = &c0.container[..max_len];
                let c1 = &c1.container[..max_len];
                let c2 = &c2.container[..max_len];
                let c3 = &c3.container[..max_len];

                let mut data = Vec::with_capacity(max_len * 4);

                for i in 0..max_len {
                    data.push(c0[i].round() as u8);
                    data.push(c1[i].round() as u8);
                    data.push(c2[i].round() as u8);
                    data.push(c3[i].round() as u8);
                }

                data
            }
            _ => unreachable!(),
        }
    } else {
        // Slow path that also requires us to scale to 8 bit.
        let mut buf = Vec::with_capacity(max_len * components.len());

        let mul_factor = ((1 << 8) - 1) as f32;

        for sample in 0..max_len {
            for channel in components.iter() {
                buf.push(
                    ((channel.container[sample] / ((1 << channel.bit_depth) - 1) as f32)
                        * mul_factor)
                        .round() as u8,
                )
            }
        }

        buf
    }
}

fn resolve_color_space(
    image: &mut DecodedImage,
    bit_depth: u8,
) -> Result<ColorSpace, &'static str> {
    let cs = match &image
        .boxes
        .color_specification
        .as_ref()
        .unwrap()
        .color_space
    {
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
                // TODO: Actually implement this.
                EnumeratedColorspace::EsRgb => ColorSpace::RGB,
                EnumeratedColorspace::Greyscale => ColorSpace::Gray,
                EnumeratedColorspace::Sycc => {
                    sycc_to_rgb(&mut image.decoded.components, bit_depth)
                        .ok_or("failed to convert image from sycc to RGB")?;

                    ColorSpace::RGB
                }
                _ => return Err("unsupported JP2 image"),
            }
        }
        jp2::colr::ColorSpace::Icc(icc) => {
            if let Some(metadata) = ICCMetadata::from_data(icc) {
                ColorSpace::Icc {
                    profile: icc.clone(),
                    num_components: metadata.color_space.num_components(),
                }
            } else {
                // See OPENJPEG test orb-blue10-lin-jp2.jp2. They seem to
                // assume RGB in this case (even though the image has 4
                // components with no opacity channel, they assume RGBA instead
                // of CMYK).
                ColorSpace::RGB
            }
        }
        jp2::colr::ColorSpace::Unknown => match image.decoded.components.len() {
            1 => ColorSpace::Gray,
            3 => ColorSpace::RGB,
            4 => ColorSpace::CMYK,
            _ => return Err("JP2 image has unsupported color space"),
        },
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
