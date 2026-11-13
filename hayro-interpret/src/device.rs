use crate::font::GlyphRun;
use crate::x_object::soft_mask::SoftMask;
use crate::{BlendMode, ClipPath, FillRule, Image};
use crate::{DrawMode, DrawProps, ImageDrawProps};
use hayro_syntax::object::{Dict, Name, ObjectIdentifier};
use kurbo::{BezPath, Rect, Shape};
use std::ops::Range;

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
        isolated: bool,
        knockout: bool,
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
    /// The tag is the marked content tag (e.g. b"P", b"Span"). The properties are the property list
    /// dictionary.
    fn begin_marked_content(&mut self, _tag: &[u8], _properties: Option<&Dict<'_>>) {}
    /// Called at the end of a marked content sequence (EMC).
    fn end_marked_content(&mut self) {}
    /// Called before a content stream other than the page's own is interpreted.
    ///
    /// Until the matching [`Device::end_content`], the spans passed to [`Device::operator`]
    /// refer to the decoded data of that stream. Content streams can be nested, for example
    /// when a form `XObject` draws another one.
    fn begin_content(&mut self, _source: &ContentSource<'_>) {}
    /// Called after the content stream of the matching [`Device::begin_content`] was
    /// interpreted.
    fn end_content(&mut self) {}
    /// Called before an instruction is interpreted, with the range of its operands and operator
    /// in the decoded data of the content stream, as returned by
    /// [`TypedIter::next_with_span`](hayro_syntax::content::TypedIter::next_with_span).
    ///
    /// For the page's own content stream, the range refers to
    /// [`Page::page_stream`](hayro_syntax::page::Page::page_stream).
    fn operator(&mut self, _span: Range<usize>) {}
}

/// A content stream other than the page's own, see [`Device::begin_content`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ContentSource<'a> {
    /// A form `XObject` drawn with `Do`.
    FormXObject {
        /// The name of the `XObject` in the resources.
        name: Name<'a>,
        /// The stream of the `XObject`.
        stream: ObjectIdentifier,
    },
    /// The appearance stream of an annotation.
    Appearance {
        /// The annotation dictionary, as given by [`Dict::obj_id`].
        annotation: Option<ObjectIdentifier>,
        /// The appearance stream.
        stream: ObjectIdentifier,
    },
    /// The pattern cell of a tiling pattern.
    TilingPattern {
        /// The name of the pattern in the resources.
        name: Name<'a>,
        /// The stream of the pattern.
        stream: ObjectIdentifier,
    },
    /// The glyph procedure of a Type 3 font.
    Type3Glyph {
        /// The font dictionary, as given by [`Dict::obj_id`].
        font: Option<ObjectIdentifier>,
        /// The character code of the glyph.
        code: u32,
        /// The stream of the glyph procedure.
        stream: ObjectIdentifier,
    },
    /// The group of a soft mask.
    SoftMask {
        /// The stream of the group.
        stream: ObjectIdentifier,
    },
}

/// A device that discards all drawing operations.
pub struct DummyDevice;

impl Device<'_> for DummyDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'_>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(
        &mut self,
        _: f32,
        _: Option<SoftMask<'_>>,
        _: BlendMode,
        _: bool,
        _: bool,
    ) {
    }
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, '_>, _: DrawProps<'_>, _: &DrawMode) {}
    fn draw_image(&mut self, _: Image<'_, '_>, _: ImageDrawProps<'_>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}
