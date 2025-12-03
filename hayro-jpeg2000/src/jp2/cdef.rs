//! The channel definition box (colr), defined in I.5.3.6.

use crate::jp2::{ChannelAssociation, ChannelDefinition, ChannelType, ImageMetadata};
use crate::reader::BitReader;

pub(crate) fn parse(metadata: &mut ImageMetadata, data: &[u8]) -> Option<()> {
    if data.len() < 2 {
        return None;
    }

    let mut reader = BitReader::new(data);
    let count = reader.read_u16()? as usize;
    let mut definitions = Vec::with_capacity(count);

    for _ in 0..count {
        let channel_index = reader.read_u16()?;
        let channel_type = reader.read_u16()?;
        let association = reader.read_u16()?;

        definitions.push(ChannelDefinition {
            channel_index,
            channel_type: ChannelType::from_raw(channel_type),
            association: ChannelAssociation::from_raw(association),
        });
    }

    self.channel_definitions = definitions;
    Some(())
}

/// Association between codestream components/channels and their semantic role.
#[derive(Debug, Clone)]
pub struct ChannelDefinition {
    pub channel_index: u16,
    pub channel_type: ChannelType,
    pub association: ChannelAssociation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    Colour,
    Opacity,
}

impl ChannelType {
    fn from_raw(value: u16) -> Self {
        match value {
            0 => ChannelType::Colour,
            1 => ChannelType::Opacity,
            2 => ChannelType::PremultipliedOpacity,
            u16::MAX => ChannelType::Unspecified,
            v => ChannelType::Reserved(v),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAssociation {
    WholeImage,
    Colour(u16),
    Unspecified,
}

impl ChannelAssociation {
    fn from_raw(value: u16) -> Self {
        match value {
            0 => ChannelAssociation::WholeImage,
            u16::MAX => ChannelAssociation::Unspecified,
            v => ChannelAssociation::Colour(v),
        }
    }
}