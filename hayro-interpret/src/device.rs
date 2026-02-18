use crate::font::Glyph;
use crate::soft_mask::SoftMask;
use crate::{BlendMode, ClipPath, Image};
use crate::{GlyphDrawMode, Paint, PathDrawMode};
use kurbo::{Affine, BezPath};

/// A trait for a device that can be used to process PDF drawing instructions.
pub trait Device<'a> {
    /// Set the properties for future stroking operations.
    /// Set a soft mask to be used for future drawing instructions.
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>);
    /// Set the blend mode that should be used for rendering operations.
    fn set_blend_mode(&mut self, blend_mode: BlendMode);
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
    fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'a>>,
        blend_mode: BlendMode,
    );
    /// Draw a glyph.
    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        // TODO: Move this into outline glyph.
        draw_mode: &GlyphDrawMode,
    );
    /// Draw an image.
    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine);
    /// Pop the last clip path from the clip stack.
    fn pop_clip_path(&mut self);
    /// Pop the last transparency group from the blend stack.
    fn pop_transparency_group(&mut self);
    /// Called at the beginning of a marked content sequence (BMC/BDC).
    ///
    /// The tag is the marked content tag (e.g. b"P", b"Span"). The mcid is
    /// the marked content identifier from the properties dict, if present.
    fn begin_marked_content(&mut self, _tag: &[u8], _mcid: Option<i32>) {}
    /// Called at the end of a marked content sequence (EMC).
    fn end_marked_content(&mut self) {}
}
