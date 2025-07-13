#[derive(Clone)]
pub struct RgbData {
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub interpolate: bool,
}

#[derive(Clone)]
pub struct AlphaData {
    pub stencil_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub interpolate: bool,
}
