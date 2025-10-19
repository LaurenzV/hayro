use crate::boxes::{
    COLOUR_SPECIFICATION, FILE_TYPE, IMAGE_HEADER, JP2_HEADER, JP2_SIGNATURE, read_box,
};
use crate::reader::Reader;

pub mod boxes;
pub mod reader;

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

pub fn read(data: &[u8]) -> Option<ImageMetadata> {
    let mut reader = Reader::new(data);
    let signature_box = read_box(&mut reader)?;

    if signature_box.box_type != JP2_SIGNATURE {
        return None;
    }

    let file_type_box = read_box(&mut reader)?;

    if file_type_box.box_type != FILE_TYPE {
        return None;
    }

    // Read boxes until we find the JP2 Header box
    while !reader.at_end() {
        let current_box = read_box(&mut reader)?;

        if current_box.box_type == JP2_HEADER {
            // Parse the JP2 Header box (superbox)
            let mut metadata = ImageMetadata {
                height: 0,
                width: 0,
                num_components: 0,
                bits_per_component: 0,
                compression_type: 0,
                colourspace_unknown: 0,
                has_intellectual_property: 0,
                colour_method: None,
                enumerated_colourspace: None,
                icc_profile: None,
            };

            let mut jp2h_reader = Reader::new(current_box.data);

            // Read child boxes within JP2 Header box
            while !jp2h_reader.at_end() {
                let child_box = read_box(&mut jp2h_reader)?;

                match child_box.box_type {
                    IMAGE_HEADER => {
                        metadata.parse_ihdr(child_box.data)?;
                    }
                    COLOUR_SPECIFICATION => {
                        metadata.parse_colr(child_box.data)?;
                    }
                    _ => {
                        // Ignore other boxes for now
                    }
                }
            }

            return Some(metadata);
        }
    }

    None
}
