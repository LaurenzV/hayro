//! The channel definition box (cdef), defined in I.5.3.6.

use crate::jp2::ImageBoxes;
use crate::reader::BitReader;

pub(crate) fn parse(boxes: &mut ImageBoxes, data: &[u8]) -> Option<()> {
    let mut reader = BitReader::new(data);
    let count = reader.read_u16()? as usize;
    let mut definitions = Vec::with_capacity(count);

    if count == 0 {
        return None;
    }

    for _ in 0..count {
        let channel_index = reader.read_u16()?;
        let channel_type = reader.read_u16()?;
        let association = reader.read_u16()?;

        definitions.push(ChannelDefinition {
            channel_index,
            channel_type: ChannelType::from_raw(channel_type)?,
            _association: ChannelAssociation::from_raw(association)?,
        });
    }

    definitions.sort_by(|a, b| a.channel_index.cmp(&b.channel_index));

    // Ensure channel indices increases in steps of 1, starting from 0.
    for (idx, def) in definitions.iter().enumerate() {
        if def.channel_index as usize != idx {
            return None;
        }
    }

    boxes.channel_definition = Some(ChannelDefinitionBox {
        channel_definitions: definitions,
    });

    Some(())
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
            0 => Some(ChannelType::Colour),
            1 => Some(ChannelType::Opacity),
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
            0 => Some(ChannelAssociation::WholeImage),
            // Unspecified.
            u16::MAX => None,
            v => Some(ChannelAssociation::Colour(v)),
        }
    }
}
