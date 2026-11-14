use crate::CacheKey;
use crate::color::Color;
use crate::pattern::Pattern;
use crate::util::hash128;
use crate::x_object::ImageXObject;
use crate::x_object::soft_mask::SoftMask;
use hayro_syntax::object::Stream;
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

/// A stencil image.
pub struct StencilImage<'a, 'b> {
    pub(crate) paint: Paint<'a>,
    pub(crate) image_xobject: ImageXObject<'b>,
}

impl<'a, 'b> StencilImage<'a, 'b> {
    /// Run `func` with the decoded stencil data, optionally hinting at the desired resolution.
    pub fn with_stencil(
        &self,
        func: impl FnOnce(LumaData, &Paint<'a>),
        target_dimension: Option<(u32, u32)>,
    ) {
        if let Some(decoded) = self.image_xobject.decoded_mask(target_dimension) {
            func(decoded.luma, &self.paint);
        }
    }

    /// Return the underlying image stream.
    pub fn stream(&self) -> &Stream<'b> {
        self.image_xobject.stream()
    }

    // Hidden: clients should use the dimensions of the decoded data.
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
    /// Run `func` with the decoded RGB and alpha data, optionally hinting at the desired resolution.
    pub fn with_rgba(
        &self,
        func: impl FnOnce(ImageData, Option<LumaData>),
        target_dimension: Option<(u32, u32)>,
    ) {
        if let Some(decoded) = self.0.decoded_image(target_dimension) {
            func(decoded.image, decoded.alpha);
        }
    }

    /// Return the underlying image stream.
    pub fn stream(&self) -> &Stream<'_> {
        self.0.stream()
    }

    // Hidden: clients should use the dimensions of the decoded data.
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
    // Hidden: clients should use the dimensions of the decoded data.
    #[doc(hidden)]
    pub fn width(&self) -> u32 {
        match self {
            Image::Stencil(s) => s.width(),
            Image::Raster(r) => r.width(),
        }
    }

    // Hidden: clients should use the dimensions of the decoded data.
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
    /// The pixel data, of length `width * height * 3`.
    pub data: Vec<u8>,
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
    /// Whether the image should be interpolated.
    pub interpolate: bool,
    /// The x and y factors to scale the image by so that it covers its intended area.
    pub scale_factors: (f32, f32),
}

/// A structure holding 1-channel luma data.
#[derive(Clone)]
pub struct LumaData {
    /// The pixel data, of length `width * height`.
    pub data: Vec<u8>,
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
    /// Whether the image should be interpolated.
    pub interpolate: bool,
    /// The x and y factors to scale the image by so that it covers its intended area.
    pub scale_factors: (f32, f32),
}

/// The color data of a raster image, either 3-channel RGB or 1-channel luma.
#[derive(Clone)]
pub enum ImageData {
    /// 3-channel RGB data.
    Rgb(RgbData),
    /// 1-channel grayscale data.
    Luma(LumaData),
}

impl ImageData {
    /// The width of the image.
    pub fn width(&self) -> u32 {
        match self {
            Self::Rgb(d) => d.width,
            Self::Luma(d) => d.width,
        }
    }

    /// The height of the image.
    pub fn height(&self) -> u32 {
        match self {
            Self::Rgb(d) => d.height,
            Self::Luma(d) => d.height,
        }
    }

    /// Whether the image should be interpolated.
    pub fn interpolate(&self) -> bool {
        match self {
            Self::Rgb(d) => d.interpolate,
            Self::Luma(d) => d.interpolate,
        }
    }

    /// The scaling factors of the image.
    pub fn scale_factors(&self) -> (f32, f32) {
        match self {
            Self::Rgb(d) => d.scale_factors,
            Self::Luma(d) => d.scale_factors,
        }
    }
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

/// Properties for a painted drawing operation.
#[derive(Clone)]
pub struct DrawProps<'a> {
    /// The transform.
    pub transform: Affine,
    /// The paint.
    pub paint: Paint<'a>,
    /// The soft mask.
    pub soft_mask: Option<SoftMask<'a>>,
    /// The blend mode.
    pub blend_mode: BlendMode,
}

/// Properties for an image drawing operation.
#[derive(Clone)]
pub struct ImageDrawProps<'a> {
    /// The transform.
    pub transform: Affine,
    /// The soft mask.
    pub soft_mask: Option<SoftMask<'a>>,
    /// The blend mode.
    pub blend_mode: BlendMode,
}

/// The draw mode.
#[derive(Clone, Debug)]
pub enum DrawMode {
    /// Draw using a fill.
    Fill(FillRule),
    /// Draw using a stroke.
    Stroke(StrokeProps),
    /// Draw using both fill and stroke.
    FillAndStroke(FillRule, StrokeProps),
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
