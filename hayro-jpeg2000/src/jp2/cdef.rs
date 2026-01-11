//! The channel definition box (cdef), defined in I.5.3.6.

use alloc::vec::Vec;

use crate::error::{FormatError, Result, bail};
use crate::jp2::ImageBoxes;
use crate::reader::BitReader;

pub(crate) fn parse(boxes: &mut ImageBoxes, data: &[u8]) -> Result<()> {
    let mut reader = BitReader::new(data);
    let count = reader.read_u16().ok_or(FormatError::InvalidBox)? as usize;
    let mut definitions = Vec::with_capacity(count);

    if count == 0 {
        bail!(FormatError::InvalidBox);
    }

    for _ in 0..count {
        let channel_index = reader.read_u16().ok_or(FormatError::InvalidBox)?;
        let channel_type = reader.read_u16().ok_or(FormatError::InvalidBox)?;
        let association = reader.read_u16().ok_or(FormatError::InvalidBox)?;

        definitions.push(ChannelDefinition {
            channel_index,
            channel_type: ChannelType::from_raw(channel_type).ok_or(FormatError::InvalidBox)?,
            _association: ChannelAssociation::from_raw(association)
                .ok_or(FormatError::InvalidBox)?,
        });
    }

    definitions.sort_by(|a, b| a.channel_index.cmp(&b.channel_index));

    // Ensure channel indices increases in steps of 1, starting from 0.
    for (idx, def) in definitions.iter().enumerate() {
        if def.channel_index as usize != idx {
            bail!(FormatError::InvalidBox);
        }
    }

    boxes.channel_definition = Some(ChannelDefinitionBox {
        channel_definitions: definitions,
    });

    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelDefinitionBox {
    pub(crate) channel_definitions: Vec<ChannelDefinition>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelDefinition {
    pub(crate) channel_index: u16,
    pub(crate) channel_type: ChannelType,
    pub(crate) _association: ChannelAssociation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelType {
    Colour,
    Opacity,
}

impl ChannelType {
    fn from_raw(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::Colour),
            1 => Some(Self::Opacity),
            // We don't support the others.
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelAssociation {
    WholeImage,
    Colour(u16),
}

impl ChannelAssociation {
    fn from_raw(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::WholeImage),
            // Unspecified.
            u16::MAX => None,
            v => Some(Self::Colour(v)),
        }
    }
}
