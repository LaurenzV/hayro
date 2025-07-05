use crate::clip_path::ClipPath;
use crate::font::Glyph;
use crate::image::{RgbaImage, StencilImage};
use crate::paint::Paint;
use crate::soft_mask::SoftMask;
use crate::{FillProps, StrokeProps};
use kurbo::{Affine, BezPath};

pub trait Device {
    fn set_transform(&mut self, affine: Affine);
    fn stroke_path(&mut self, path: &BezPath, paint: &Paint);
    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps);
    fn set_soft_mask(&mut self, mask: Option<SoftMask>);
    fn fill_path(&mut self, path: &BezPath, paint: &Paint);
    fn set_fill_properties(&mut self, fill_props: &FillProps);
    fn push_clip_path(&mut self, clip_path: &ClipPath);
    fn push_transparency_group(&mut self, opacity: f32);
    fn fill_glyph(&mut self, glyph: &Glyph<'_>, paint: &Paint);
    fn stroke_glyph(&mut self, glyph: &Glyph<'_>, paint: &Paint);
    fn draw_rgba_image(&mut self, image: RgbaImage);
    fn draw_stencil_image(&mut self, stencil: StencilImage, paint: &Paint);
    fn pop_clip_path(&mut self);
    fn pop_transparency_group(&mut self);
}
