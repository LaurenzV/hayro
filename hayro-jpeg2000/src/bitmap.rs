use crate::ImageMetadata;

#[derive(Debug)]
pub struct Bitmap {
    pub channels: Vec<ChannelData>,
    pub metadata: ImageMetadata,
}
