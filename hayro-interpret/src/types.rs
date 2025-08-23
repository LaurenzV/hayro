use crate::CacheKey;
use crate::color::Color;
use crate::pattern::Pattern;
use crate::util::hash128;
use crate::x_object::{DecodedImageXObject, ImageXObject};
use kurbo::{BezPath, Cap, Join};
use smallvec::{SmallVec, smallvec};
use std::sync::OnceLock;

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

/// A stencil image.
pub struct StencilImage<'a> {
    image_xobject: ImageXObject<'a>,
    paint: Paint<'a>,
}

impl<'a> StencilImage<'a> {
    /// Return the stencil data of the image.
    ///
    /// Returns `None` if the data of the stencil image was invalid, in which case
    /// it should be ignored.
    ///
    /// The reason this can happen is that `hayro` can't validate the data without actually decoding
    /// it, which would be expensive.
    pub fn stencil_data(&self) -> Option<LumaData> {
        self.image_xobject
            .decoded_object()
            .and_then(|d| d.luma_data)
    }

    /// Return the paint the stencil image should be painted with.
    pub fn paint(&self) -> &Paint<'a> {
        &self.paint
    }
}

impl CacheKey for StencilImage<'_> {
    fn cache_key(&self) -> u128 {
        self.image_xobject.cache_key()
    }
}

/// A raster image.
pub struct RasterImage<'a>(pub(crate) ImageXObject<'a>);

impl RasterImage<'_> {
    /// Returns the image as RGB.
    ///
    /// Returns `None` if the image couldn't be decoded because it is invalid, in which case
    /// it should be ignored.
    ///
    /// The reason this can happen is that `hayro` can't validate the data without actually decoding
    /// it, which would be expensive.
    pub fn rgba_channels(&self) -> (Option<RgbData>, Option<LumaData>) {
        let decoded = self.0.decoded_object();

        if let Some(decoded) = decoded {
            (decoded.rgb_data, decoded.luma_data)
        } else {
            (None, None)
        }
    }
}

impl CacheKey for RasterImage<'_> {
    fn cache_key(&self) -> u128 {
        self.0.cache_key()
    }
}

/// A type of image.
pub enum Image<'a> {
    /// A stencil image.
    Stencil(StencilImage<'a>),
    /// A normal raster image.
    Raster(RasterImage<'a>),
}

impl<'a> CacheKey for Image<'a> {
    fn cache_key(&self) -> u128 {
        match self {
            Image::Stencil(i) => i.cache_key(),
            Image::Raster(i) => i.cache_key(),
        }
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
pub enum Paint<'a> {
    /// A solid RGBA color.
    Color(Color),
    /// A PDF pattern.
    Pattern(Box<Pattern<'a>>),
}

/// The draw mode that should be used for a path.
#[derive(Clone, Debug)]
pub enum PathDrawMode {
    /// Draw using a fill.
    Fill(FillRule),
    /// Draw using a stroke.
    Stroke(StrokeProps),
}

/// The draw mode that should be used for a glyph.
#[derive(Clone, Debug)]
pub enum GlyphDrawMode {
    /// Draw using a fill.
    Fill,
    /// Draw using a stroke.
    Stroke(StrokeProps),
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
