use crate::ClipPath;
use crate::Paint;
use crate::font::Glyph;
use crate::soft_mask::SoftMask;
use crate::{FillProps, StrokeProps};
use crate::{LumaData, RgbData};
use kurbo::{Affine, BezPath};

/// A trait for a device that can be used to process PDF drawing instructions.
pub trait Device {
    /// Set the current transform for paths, glyphs and images.
    fn set_transform(&mut self, affine: Affine);
    /// Stroke a path.
    fn stroke_path(&mut self, path: &BezPath, paint: &Paint);
    /// Set the properties for future stroking operations.
    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps);
    /// Set a soft mask to be used for future drawing instructions.
    fn set_soft_mask(&mut self, mask: Option<SoftMask>);
    /// Fill a path.
    fn fill_path(&mut self, path: &BezPath, paint: &Paint);
    /// Set the properties for future filling operations.
    fn set_fill_properties(&mut self, fill_props: &FillProps);
    /// Push a new clip path to the clip stack.
    fn push_clip_path(&mut self, clip_path: &ClipPath);
    /// Push a new transparency group to the blend stack.
    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask>);
    /// Fill a glyph.
    fn fill_glyph(&mut self, glyph: &Glyph<'_>, paint: &Paint);
    /// Stroke a glyph.
    fn stroke_glyph(&mut self, glyph: &Glyph<'_>, paint: &Paint);
    /// Draw an RGBA image.
    fn draw_rgba_image(&mut self, image: RgbData, alpha: Option<LumaData>);
    /// Draw a stencil image with the given paint.
    fn draw_stencil_image(&mut self, stencil: LumaData, paint: &Paint);
    /// Pop the last clip path from the clip stack.
    fn pop_clip_path(&mut self);
    /// Pop the last transparency group from the blend stack.
    fn pop_transparency_group(&mut self);
}
