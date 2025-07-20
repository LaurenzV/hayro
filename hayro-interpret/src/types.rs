use crate::CacheKey;
use crate::color::Color;
use crate::pattern::Pattern;
use crate::util::hash128;
use kurbo::{Affine, BezPath, Cap, Join};
use smallvec::{SmallVec, smallvec};

/// A clip path.
#[derive(Debug, Clone)]
pub struct ClipPath {
    /// The clipping path.
    pub path: BezPath,
    /// The fill rule.
    pub fill: FillRule,
}

impl CacheKey for ClipPath {
    fn cache_key(&self) -> u128 {
        hash128(&(&self.path.to_svg(), &self.fill))
    }
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
    Pattern(Box<Pattern<'a>>),
}

/// A paint.
#[derive(Clone, Debug)]
pub struct Paint<'a> {
    /// A transform to apply to the paint.
    pub paint_transform: Affine,
    /// The underlying type of paint.
    pub paint_type: PaintType<'a>,
}

/// Stroke properties.
#[derive(Clone, Debug)]
pub struct StrokeProps {
    /// The line width.
    pub line_width: f32,
    /// The line cap.
    pub line_cap: Cap,
    /// The line join.
    pub line_join: Join,
    /// The miter limit.
    pub miter_limit: f32,
    /// The dash array.
    pub dash_array: SmallVec<[f32; 4]>,
    /// The dash offset.
    pub dash_offset: f32,
}

impl Default for StrokeProps {
    fn default() -> Self {
        Self {
            line_width: 1.0,
            line_cap: Cap::Butt,
            line_join: Join::Miter,
            miter_limit: 10.0,
            dash_array: smallvec![],
            dash_offset: 0.0,
        }
    }
}

/// A fill rule.
#[derive(Clone, Debug, Copy, Hash, PartialEq, Eq)]
pub enum FillRule {
    /// Non-zero filling.
    NonZero,
    /// Even-odd filling.
    EvenOdd,
}

/// Fill properties.
#[derive(Clone, Debug)]
pub struct FillProps {
    /// The fill rule.
    pub fill_rule: FillRule,
}

impl Default for FillProps {
    fn default() -> Self {
        Self {
            fill_rule: FillRule::NonZero,
        }
    }
}
