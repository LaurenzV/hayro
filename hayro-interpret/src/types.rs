use crate::FillRule;
use crate::color::Color;
use crate::pattern::Pattern;
use kurbo::{Affine, BezPath};

/// A clip path.
#[derive(Debug, Clone)]
pub struct ClipPath {
    /// The clipping path.
    pub path: BezPath,
    /// The fill rule.
    pub fill: FillRule,
}

/// A structure holding 3-channel RGB data.
#[derive(Clone)]
pub struct RgbData {
    /// The actual data. It is guaranteed to have the length width * height * 3.
    pub data: Vec<u8>,
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
    /// Whether the image should be interpolated.
    pub interpolate: bool,
}

/// A structure holding 1-channel luma data.
#[derive(Clone)]
pub struct LumaData {
    /// The actual data. It is guaranteed to have the length width * height.
    pub data: Vec<u8>,
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
    /// Whether the image should be interpolated.
    pub interpolate: bool,
}

/// A type of paint.
#[derive(Clone, Debug)]
pub enum PaintType<'a> {
    /// A solid RGBA color.
    Color(Color),
    /// A PDF pattern.
    Pattern(Pattern<'a>),
}

/// A paint.
#[derive(Clone, Debug)]
pub struct Paint<'a> {
    /// A transform to apply to the paint.
    pub paint_transform: Affine,
    /// The underlying type of paint.
    pub paint_type: PaintType<'a>,
}
