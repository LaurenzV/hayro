use crate::CacheKey;
use crate::color::Color;
use crate::pattern::Pattern;
use crate::util::hash128;
use crate::x_object::ImageXObject;
use kurbo::{BezPath, Cap, Join};
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

/// A stencil image.
pub struct StencilImage<'a, 'b> {
    pub(crate) paint: Paint<'a>,
    pub(crate) image_xobject: ImageXObject<'b>,
}

impl<'a, 'b> StencilImage<'a, 'b> {
    /// Perform some operation with the stencil data of the image.
    ///
    /// The second argument allows you to give the image decoder a hint for
    /// what resolution of the image you want to have. Note that this does not
    /// mean that the resulting image will have that dimension. Instead, it allows
    /// the image decoder to extract a lower-resolution version of the image in
    /// certain cases.
    pub fn with_stencil(
        &self,
        func: impl FnOnce(LumaData, &Paint<'a>),
        target_dimension: Option<(u32, u32)>,
    ) {
        if let Some(luma) = self
            .image_xobject
            .decoded_object(target_dimension)
            .and_then(|d| d.luma_data)
        {
            func(luma, &self.paint);
        }
    }

    // These are hidden since clients are supposed to call get the
    // width/height from `LumaData` instead.
    #[doc(hidden)]
    pub fn width(&self) -> u32 {
        self.image_xobject.width()
    }

    #[doc(hidden)]
    pub fn height(&self) -> u32 {
        self.image_xobject.height()
    }
}

impl CacheKey for StencilImage<'_, '_> {
    fn cache_key(&self) -> u128 {
        self.image_xobject.cache_key()
    }
}

/// A raster image.
pub struct RasterImage<'a>(pub(crate) ImageXObject<'a>);

impl RasterImage<'_> {
    /// Perform some operation with the RGB and alpha channel of the image.
    ///
    /// The second argument allows you to give the image decoder a hint for
    /// what resolution of the image you want to have. Note that this does not
    /// mean that the resulting image will have that dimension. Instead, it allows
    /// the image decoder to extract a lower-resolution version of the image in
    /// certain cases.
    pub fn with_rgba(
        &self,
        func: impl FnOnce(RgbData, Option<LumaData>),
        target_dimension: Option<(u32, u32)>,
    ) {
        let decoded = self.0.decoded_object(target_dimension);

        if let Some(decoded) = decoded
            && let Some(rgb) = decoded.rgb_data
        {
            func(rgb, decoded.luma_data);
        }
    }

    // These are hidden since clients are supposed to call get the
    // width/height from `LumaData` instead.
    #[doc(hidden)]
    pub fn width(&self) -> u32 {
        self.0.width()
    }

    #[doc(hidden)]
    pub fn height(&self) -> u32 {
        self.0.height()
    }
}

impl CacheKey for RasterImage<'_> {
    fn cache_key(&self) -> u128 {
        self.0.cache_key()
    }
}

/// A type of image.
pub enum Image<'a, 'b> {
    /// A stencil image.
    Stencil(StencilImage<'a, 'b>),
    /// A normal raster image.
    Raster(RasterImage<'b>),
}

impl Image<'_, '_> {
    // These are hidden since clients are supposed to call get the
    // width/height from `LumaData/RgbData` instead.
    #[doc(hidden)]
    pub fn width(&self) -> u32 {
        match self {
            Image::Stencil(s) => s.width(),
            Image::Raster(r) => r.width(),
        }
    }

    // These are hidden since clients are supposed to call get the
    // width/height from `LumaData/RgbData` instead.
    #[doc(hidden)]
    pub fn height(&self) -> u32 {
        match self {
            Image::Stencil(s) => s.height(),
            Image::Raster(r) => r.height(),
        }
    }
}

impl CacheKey for Image<'_, '_> {
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
    /// Additional scaling factors to apply to the image.
    ///
    /// In most cases, those factors will just be 1.0, and you can
    /// ignore them. There are two situations in which they will not be equal
    /// to 1:
    /// 1) The PDF provided wrong metadata about the width/height of the image,
    ///    which needs to be corrected
    /// 2) A lower resolution of the image was requested, in which case it needs
    ///    to be scaled up so that it still covers the same area.
    ///
    /// The first number indicates the x scaling factor, the second number the
    /// y scaling factor.
    pub scale_factors: (f32, f32),
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
    /// Additional scaling factors to apply to the image.
    ///
    /// In most cases, those factors will just be 1.0, and you can
    /// ignore them. There are two situations in which they will not be equal
    /// to 1:
    /// 1) The PDF provided wrong metadata about the width/height of the image,
    ///    which needs to be corrected
    /// 2) A lower resolution of the image was requested, in which case it needs
    ///    to be scaled up so that it still covers the same area.
    ///
    /// The first number indicates the x scaling factor, the second number the
    /// y scaling factor.
    pub scale_factors: (f32, f32),
}

/// A type of paint.
#[derive(Clone, Debug)]
pub enum Paint<'a> {
    /// A solid RGBA color.
    Color(Color),
    /// A PDF pattern.
    Pattern(Box<Pattern<'a>>),
}

impl CacheKey for Paint<'_> {
    fn cache_key(&self) -> u128 {
        match self {
            Paint::Color(c) => {
                // TODO: We should actually cache the color with color space etc., not just the
                // RGBA8 version.
                hash128(&c.to_rgba().to_rgba8())
            }
            Paint::Pattern(p) => p.cache_key(),
        }
    }
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
    /// Invisible text (for text extraction but not visual rendering).
    Invisible,
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

/// A blend mode.
#[derive(Clone, Debug, Copy, Hash, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Normal blend mode (default).
    #[default]
    Normal,
    /// Multiply blend mode.
    Multiply,
    /// Screen blend mode.
    Screen,
    /// Overlay blend mode.
    Overlay,
    /// Darken blend mode.
    Darken,
    /// Lighten blend mode.
    Lighten,
    /// `ColorDodge` blend mode.
    ColorDodge,
    /// `ColorBurn` blend mode.
    ColorBurn,
    /// `HardLight` blend mode.
    HardLight,
    /// `SoftLight` blend mode.
    SoftLight,
    /// Difference blend mode.
    Difference,
    /// Exclusion blend mode.
    Exclusion,
    /// Hue blend mode.
    Hue,
    /// Saturation blend mode.
    Saturation,
    /// Color blend mode.
    Color,
    /// Luminosity blend mode.
    Luminosity,
}
