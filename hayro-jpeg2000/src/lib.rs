// TODO: Remove
#![allow(warnings)]

use crate::bitmap::Bitmap;
use crate::boxes::{
    COLOUR_SPECIFICATION, CONTIGUOUS_CODESTREAM, FILE_TYPE, IMAGE_HEADER, JP2_HEADER,
    JP2_SIGNATURE, read_box,
};
use hayro_common::byte::Reader;

mod arithmetic_decoder;
pub mod bitmap;
pub(crate) mod bitplane;
pub mod boxes;
mod codestream;
mod dequantize;
pub(crate) mod idwt;
mod packet;
mod progression;
mod tag_tree;
mod tile;

/// Image metadata extracted from JP2 Header box.
#[derive(Debug, Clone)]
pub struct ImageMetadata {
    /// Image area height in reference grid points.
    pub height: u32,
    /// Image area width in reference grid points.
    pub width: u32,
    /// Number of components.
    pub num_components: u16,
    /// Bits per component (0-127 = actual bit depth - 1, high bit indicates signed).
    /// Value of 255 indicates components vary in bit depth.
    pub bits_per_component: u8,
    /// Intellectual property flag (0 = no IPR box, 1 = contains IPR box).
    pub has_intellectual_property: u8,
    /// Colour specification method (1 = enumerated, 2 = ICC profile).
    pub colour_method: Option<u8>,
    /// Enumerated colourspace (if colour_method = 1).
    pub enumerated_colourspace: Option<u32>,
    /// ICC profile data (if colour_method = 2).
    pub icc_profile: Option<Vec<u8>>,
}

impl ImageMetadata {
    /// Parse Image Header box (ihdr) data.
    fn parse_ihdr(&mut self, data: &[u8]) -> Option<()> {
        if data.len() < 14 {
            return None;
        }

        let mut reader = Reader::new(data);

        self.height = reader.read_u32()?;
        self.width = reader.read_u32()?;
        self.num_components = reader.read_u16()?;
        self.bits_per_component = reader.read_byte()?;
        let _compression_type = reader.read_byte()?;
        let _colorspace_unknown = reader.read_byte()?;
        let _has_intellectual_property = reader.read_byte()?;

        Some(())
    }

    /// Parse Colour Specification box (colr) data.
    fn parse_colr(&mut self, data: &[u8]) -> Option<()> {
        if data.len() < 3 {
            return None;
        }

        let mut reader = Reader::new(data);

        let meth = reader.read_byte()?;
        let _prec = reader.read_byte()?; // Reserved, ignored
        let _approx = reader.read_byte()?; // Reserved, ignored

        self.colour_method = Some(meth);

        match meth {
            1 => {
                // Enumerated colourspace
                self.enumerated_colourspace = Some(reader.read_u32()?);
            }
            2 => {
                // ICC profile
                let profile_data = reader.tail()?.to_vec();
                self.icc_profile = Some(profile_data);
            }
            _ => {
                // Unknown method, ignore
            }
        }

        Some(())
    }
}

pub fn read(data: &[u8]) -> Result<Bitmap, &'static str> {
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
        let current_box = read_box(&mut reader).ok_or("failed to read JP2 box")?;

        if current_box.box_type == JP2_HEADER {
            // Parse the JP2 Header box (superbox)
            let mut image_metadata = ImageMetadata {
                height: 0,
                width: 0,
                num_components: 0,
                bits_per_component: 0,
                has_intellectual_property: 0,
                colour_method: None,
                enumerated_colourspace: None,
                icc_profile: None,
            };

            let mut jp2h_reader = Reader::new(current_box.data);

            // Read child boxes within JP2 Header box
            while !jp2h_reader.at_end() {
                let child_box = read_box(&mut jp2h_reader).ok_or("failed to read JP2 box")?;

                match child_box.box_type {
                    IMAGE_HEADER => {
                        image_metadata
                            .parse_ihdr(child_box.data)
                            .ok_or("failed to parse image header")?;
                    }
                    COLOUR_SPECIFICATION => {
                        image_metadata
                            .parse_colr(child_box.data)
                            .ok_or("failed to parse colour")?;
                    }
                    _ => {
                        // eprintln!("ignoring box {}", tag_to_string(child_box.box_type));
                    }
                }
            }

            metadata = Ok(image_metadata);
        } else if current_box.box_type == CONTIGUOUS_CODESTREAM {
            channels = Ok(codestream::read(current_box.data)?);
        } else {
            // eprintln!("ignoring outer box {}", tag_to_string(current_box.box_type));
        }
    }

    Ok(Bitmap {
        channels: channels?,
        metadata: metadata?,
    })
}
