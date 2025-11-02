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
            ChannelContainer::U8(mut d) => {
                if self.bit_depth == 8 {
                    return d;
                }

                let old_max = ((1 << self.bit_depth) - 1) as f32;
                let new_max = ((1 << 8) - 1) as f32;

                for sample in &mut d {
                    *sample = ((*sample as f32 / old_max) * new_max) as u8;
                }

                d
            }
            _ => unimplemented!(),
        }
    }
}
