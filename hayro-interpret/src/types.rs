use kurbo::BezPath;
use crate::FillRule;

#[derive(Debug, Clone)]
pub struct ClipPath {
    pub path: BezPath,
    pub fill: FillRule,
}

#[derive(Clone)]
pub struct RgbData {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub interpolate: bool,
}

#[derive(Clone)]
pub struct LumaData {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub interpolate: bool,
}

