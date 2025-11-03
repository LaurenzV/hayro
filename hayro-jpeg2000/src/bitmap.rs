use crate::ImageMetadata;
use std::borrow::Cow;

#[derive(Debug)]
pub struct Bitmap {
    pub channels: Vec<ChannelData>,
    pub metadata: ImageMetadata,
}

#[derive(Debug, Clone)]
pub struct ChannelData {
    pub container: Vec<f32>,
    pub bit_depth: u8,
    pub is_alpha: bool,
}

impl ChannelData {
    pub fn into_8bit(self) -> Vec<u8> {
        self.container
            .into_iter()
            .map(|sample| {
                if self.bit_depth == 8 {
                    return sample.round() as u8;
                }

                (sample * 255.0 / ((1 << self.bit_depth) - 1) as f32).round() as u8
            })
            .collect()
    }
}
