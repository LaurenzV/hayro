use crate::font::GlyphRun;
use crate::x_object::soft_mask::SoftMask;
use crate::{BlendMode, ClipPath, FillRule, Image};
use crate::{DrawMode, DrawProps, ImageDrawProps};
use hayro_syntax::object::dict::keys::{ACTUAL_TEXT, MCID};
use hayro_syntax::object::{Dict, String};
use kurbo::{BezPath, Rect, Shape};

/// A trait for a device that can be used to process PDF drawing instructions.
pub trait Device<'a> {
    /// Draw a path.
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode);
    /// Push a new clip path to the clip stack.
    fn push_clip_path(&mut self, clip_path: &ClipPath);
    /// Push a rectangular clip to the clip stack.
    fn push_clip_rect(&mut self, rect: &Rect) {
        self.push_clip_path(&ClipPath {
            path: rect.to_path(0.1),
            fill: FillRule::NonZero,
        });
    }
    /// Push a new transparency group to the blend stack.
    fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'a>>,
        blend_mode: BlendMode,
    );
    /// Draw a run of positioned glyphs.
    fn draw_glyph_run(
        &mut self,
        glyph_run: &GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    );
    /// Draw an image.
    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>);
    /// Pop the last clip path or clip rectangle from the clip stack.
    fn pop_clip(&mut self);
    /// Pop the last transparency group from the blend stack.
    fn pop_transparency_group(&mut self);
    /// Draw a rectangle directly, without going through the general path pipeline.
    fn draw_rect(&mut self, rect: &Rect, props: DrawProps<'a>, draw_mode: &DrawMode) {
        self.draw_path(&rect.to_path(0.1), props, draw_mode);
    }
    /// Called at the beginning of a marked content sequence (BMC/BDC).
    ///
    /// The tag is the marked content tag (e.g. b"P", b"Span"). For BMC, the properties are empty.
    fn begin_marked_content(&mut self, _tag: &[u8], _properties: &MarkedContentProperties<'_>) {}
    /// Called at the end of a marked content sequence (EMC).
    fn end_marked_content(&mut self) {}
}

/// A device that discards all drawing operations.
pub struct DummyDevice;

impl Device<'_> for DummyDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'_>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, '_>, _: DrawProps<'_>, _: &DrawMode) {}
    fn draw_image(&mut self, _: Image<'_, '_>, _: ImageDrawProps<'_>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The property list of a marked-content sequence.
#[derive(Clone, Debug, Default)]
pub struct MarkedContentProperties<'a> {
    dict: Option<Dict<'a>>,
}

impl<'a> MarkedContentProperties<'a> {
    pub(crate) fn new(dict: Option<Dict<'a>>) -> Self {
        Self { dict }
    }

    /// The marked-content identifier, if present.
    pub fn mcid(&self) -> Option<i32> {
        self.dict.as_ref()?.get::<i32>(MCID)
    }

    /// The replacement text from `/ActualText`, if present.
    ///
    /// It is a PDF text string and needs to be decoded.
    pub fn actual_text(&self) -> Option<String<'a>> {
        self.dict.as_ref()?.get::<String<'a>>(ACTUAL_TEXT)
    }
}
