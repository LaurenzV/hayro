use crate::Paint;
use crate::StrokeProps;
use crate::font::Glyph;
use crate::soft_mask::SoftMask;
use crate::{ClipPath, FillRule};
use crate::{LumaData, RgbData};
use kurbo::{Affine, BezPath};

/// A trait for a device that can be used to process PDF drawing instructions.
pub trait Device<'a> {
    /// Stroke a path.
    fn stroke_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        stroke_props: &StrokeProps,
    );
    /// Set the properties for future stroking operations.
    /// Set a soft mask to be used for future drawing instructions.
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>);
    /// Fill a path.
    fn fill_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        fill_rule: FillRule,
    );
    /// Push a new clip path to the clip stack.
    fn push_clip_path(&mut self, clip_path: &ClipPath);
    /// Push a new transparency group to the blend stack.
    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask<'a>>);
    /// Fill a glyph.
    fn fill_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
    );
    /// Stroke a glyph.
    fn stroke_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        stroke_props: &StrokeProps,
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
