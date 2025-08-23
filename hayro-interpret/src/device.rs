use crate::ClipPath;
use crate::font::Glyph;
use crate::soft_mask::SoftMask;
use crate::{GlyphDrawMode, Paint, PathDrawMode};
use crate::{LumaData, RgbData};
use kurbo::{Affine, BezPath};

/// A trait for a device that can be used to process PDF drawing instructions.
pub trait Device<'a> {
    /// Set the properties for future stroking operations.
    /// Set a soft mask to be used for future drawing instructions.
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>);
    /// Draw a path.
    fn draw_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &PathDrawMode,
    );
    /// Push a new clip path to the clip stack.
    fn push_clip_path(&mut self, clip_path: &ClipPath);
    /// Push a new transparency group to the blend stack.
    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask<'a>>);
    /// Draw a glyph.
    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &GlyphDrawMode,
    );
    /// Draw an RGBA image.
    fn draw_rgba_image(&mut self, image: RgbData, transform: Affine, alpha: Option<LumaData>);
    /// Draw a stencil image with the given paint.
    fn draw_stencil_image(&mut self, stencil: LumaData, transform: Affine, paint: &Paint<'a>);
    /// Pop the last clip path from the clip stack.
    fn pop_clip_path(&mut self);
    /// Pop the last transparency group from the blend stack.
    fn pop_transparency_group(&mut self);
}
