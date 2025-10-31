use crate::ImageMetadata;
use std::borrow::Cow;

#[derive(Debug)]
pub struct Bitmap {
    pub channels: Vec<ChannelData>,
    pub metadata: ImageMetadata,
}

#[derive(Debug)]
pub enum ChannelContainer {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
}

#[derive(Debug)]
pub struct ChannelData {
    pub container: ChannelContainer,
    pub bit_depth: u8,
    pub is_alpha: bool,
}

impl ChannelData {
    pub fn into_8bit(self) -> Vec<u8> {
        match self.container {
            ChannelContainer::U8(d) => d,
            _ => unimplemented!(),
        }
    }
}
