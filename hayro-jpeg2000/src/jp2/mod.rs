use alloc::vec::Vec;

use crate::cancel::Cancel;
use crate::error::{FormatError, Result, bail};
use crate::j2c::ComponentData;
use crate::jp2::r#box::{FILE_TYPE, JP2_SIGNATURE};
use crate::jp2::cdef::ChannelDefinitionBox;
use crate::jp2::cmap::{ComponentMappingBox, ComponentMappingEntry, ComponentMappingType};
use crate::jp2::colr::ColorSpecificationBox;
use crate::jp2::pclr::PaletteBox;
use crate::reader::BitReader;
use crate::{DecodeSettings, Image, resolve_alpha_and_color_space};

pub(crate) mod r#box;
pub(crate) mod cdef;
pub(crate) mod cmap;
pub(crate) mod colr;
pub(crate) mod icc;
pub(crate) mod pclr;

#[derive(Debug, Clone, Default)]
pub(crate) struct ImageBoxes {
    pub(crate) color_specification: Option<ColorSpecificationBox>,
    pub(crate) channel_definition: Option<ChannelDefinitionBox>,
    pub(crate) palette: Option<PaletteBox>,
    pub(crate) component_mapping: Option<ComponentMappingBox>,
}

/// A decoded JPEG2000 image.
pub struct DecodedImage<'a> {
    pub(crate) decoded_components: &'a mut Vec<ComponentData>,
    pub(crate) boxes: ImageBoxes,
    pub(crate) cancel: Cancel<'a>,
}

pub(crate) fn parse<'a>(data: &'a [u8], mut settings: DecodeSettings) -> Result<Image<'a>> {
    let mut reader = BitReader::new(data);
    let signature_box = r#box::read(&mut reader, settings.strict).ok_or(FormatError::InvalidBox)?;

    if signature_box.box_type != JP2_SIGNATURE {
        bail!(FormatError::InvalidSignature);
    }

    let file_type_box = r#box::read(&mut reader, settings.strict).ok_or(FormatError::InvalidBox)?;

    if file_type_box.box_type != FILE_TYPE {
        bail!(FormatError::InvalidFileType);
    }

    let mut image_boxes: Option<ImageBoxes> = None;
    let mut parsed_codestream = None;

    while !reader.at_end() {
        let Some(current_box) = r#box::read(&mut reader, settings.strict) else {
            if settings.strict {
                bail!(FormatError::InvalidBox);
            }

            break;
        };

        match current_box.box_type {
            r#box::JP2_HEADER => {
                let mut boxes = ImageBoxes::default();

                let mut jp2h_reader = BitReader::new(current_box.data);

                while !jp2h_reader.at_end() {
                    let child_box = r#box::read(&mut jp2h_reader, settings.strict)
                        .ok_or(FormatError::InvalidBox)?;

                    match child_box.box_type {
                        r#box::CHANNEL_DEFINITION => {
                            if cdef::parse(&mut boxes, child_box.data).is_err() && settings.strict {
                                bail!(FormatError::InvalidBox);
                            }
                        }
                        r#box::COLOUR_SPECIFICATION => {
                            colr::parse(&mut boxes, child_box.data)?;
                        }
                        r#box::PALETTE => {
                            if pclr::parse(&mut boxes, child_box.data).is_err() && settings.strict {
                                bail!(FormatError::InvalidBox);
                            }

                            // Reduced-resolution decoding would corrupt palette indices.
                            settings.target_resolution = None;
                        }
                        r#box::COMPONENT_MAPPING => {
                            cmap::parse(&mut boxes, child_box.data)?;
                        }
                        _ => {}
                    }
                }

                image_boxes = Some(boxes);
            }
            r#box::CONTIGUOUS_CODESTREAM => {
                parsed_codestream = Some(crate::j2c::parse_raw(current_box.data, &settings)?);
            }
            _ => {}
        }
    }

    let mut image_boxes = image_boxes.ok_or(FormatError::InvalidBox)?;
    let parsed_codestream = parsed_codestream.ok_or(FormatError::MissingCodestream)?;

    if let Some(palette) = image_boxes.palette.as_ref()
        && image_boxes.component_mapping.is_none()
    {
        // Some files have a pclr box without cmap; map every channel through the palette.
        let mappings = (0..palette.columns.len())
            .map(|i| ComponentMappingEntry {
                component_index: 0,
                mapping_type: ComponentMappingType::Palette { column: i as u8 },
            })
            .collect::<Vec<_>>();

        image_boxes.component_mapping = Some(ComponentMappingBox { entries: mappings });
    }

    let (color_space, has_alpha) =
        resolve_alpha_and_color_space(&image_boxes, &parsed_codestream.header, &settings)?;

    Ok(Image {
        codestream: parsed_codestream.data,
        header: parsed_codestream.header,
        boxes: image_boxes,
        settings,
        color_space,
        has_alpha,
    })
}
