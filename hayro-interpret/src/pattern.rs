//! PDF patterns.

use crate::cache::Cache;
use crate::color::{Color, ColorSpace};
use crate::context::Context;
use crate::device::Device;
use crate::font::Glyph;
use crate::interpret::state::{State, TextState};
use crate::shading::Shading;
use crate::soft_mask::SoftMask;
use crate::util::{Float32Ext, hash128};
use crate::{BlendMode, CacheKey, ClipPath, GlyphDrawMode, Image, PathDrawMode};
use crate::{FillRule, InterpreterSettings, Paint, interpret};
use hayro_syntax::content::TypedIter;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Rect;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{
    BBOX, EXT_G_STATE, MATRIX, PAINT_TYPE, RESOURCES, SHADING, X_STEP, Y_STEP,
};
use hayro_syntax::object::{Object, dict_or_stream};
use hayro_syntax::page::Resources;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, Shape};
use log::warn;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

/// A PDF pattern.
#[derive(Debug, Clone)]
pub enum Pattern<'a> {
    /// A shading pattern.
    Shading(ShadingPattern),
    /// A tiling pattern.
    Tiling(Box<TilingPattern<'a>>),
}

impl<'a> Pattern<'a> {
    pub(crate) fn new(
        object: Object<'a>,
        ctx: &Context<'a>,
        resources: &Resources<'a>,
    ) -> Option<Self> {
        if let Some(dict) = object.clone().into_dict() {
            Some(Self::Shading(ShadingPattern::new(
                &dict,
                &ctx.object_cache,
            )?))
        } else if let Some(stream) = object.clone().into_stream() {
            Some(Self::Tiling(Box::new(TilingPattern::new(
                stream, ctx, resources,
            )?)))
        } else {
            None
        }
    }

    pub(crate) fn pre_concat_transform(&mut self, transform: Affine) {
        match self {
            Self::Shading(p) => {
                p.matrix = transform * p.matrix;
                let transformed_clip_path = p.shading.clip_path.clone().map(|r| p.matrix * r);
                Arc::make_mut(&mut p.shading).clip_path = transformed_clip_path
            }
            Self::Tiling(p) => p.matrix = transform * p.matrix,
        }
    }
}

impl CacheKey for Pattern<'_> {
    fn cache_key(&self) -> u128 {
        match self {
            Self::Shading(p) => p.cache_key(),
            Self::Tiling(p) => p.cache_key(),
        }
    }
}

/// A shading pattern.
#[derive(Clone, Debug)]
pub struct ShadingPattern {
    /// The underlying shading of the pattern.
    pub shading: Arc<Shading>,
    /// A transformation matrix to apply prior to rendering.
    pub matrix: Affine,
}

impl ShadingPattern {
    pub(crate) fn new(dict: &Dict, cache: &Cache) -> Option<Self> {
        let shading = dict.get::<Object>(SHADING).and_then(|o| {
            let (dict, stream) = dict_or_stream(&o)?;

            Shading::new(&dict, stream.as_ref(), cache)
        })?;
        let matrix = dict
            .get::<[f64; 6]>(MATRIX)
            .map(Affine::new)
            .unwrap_or_default();

        if dict.contains_key(EXT_G_STATE) {
            warn!("shading patterns with ext_g_state are not supported yet");
        }

        Some(Self {
            shading: Arc::new(shading),
            matrix,
        })
    }
}

impl CacheKey for ShadingPattern {
    fn cache_key(&self) -> u128 {
        hash128(&(self.shading.cache_key(), self.matrix.cache_key()))
    }
}

/// A tiling pattern.
#[derive(Clone)]
pub struct TilingPattern<'a> {
    cache_key: u128,
    ctx_bbox: Rect,
    /// The bbox of the tiling pattern.
    pub bbox: Rect,
    /// The step in the x direction.
    pub x_step: f32,
    /// The step in the y direction.
    pub y_step: f32,
    /// A transformation to apply prior to rendering.
    pub matrix: Affine,
    stream: Stream<'a>,
    is_color: bool,
    pub(crate) stroke_paint: Color,
    pub(crate) non_stroking_paint: Color,
    pub(crate) state: Box<State<'a>>,
    pub(crate) parent_resources: Resources<'a>,
    pub(crate) cache: Cache,
    pub(crate) settings: InterpreterSettings,
    pub(crate) xref: &'a XRef,
}

impl Debug for TilingPattern<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("TilingPattern")
    }
}

impl<'a> TilingPattern<'a> {
    pub(crate) fn new(
        stream: Stream<'a>,
        ctx: &Context<'a>,
        resources: &Resources<'a>,
    ) -> Option<Self> {
        let cache_key = stream.cache_key();
        let dict = stream.dict();

        let bbox = dict.get::<Rect>(BBOX)?;
        let x_step = dict.get::<f32>(X_STEP)?;
        let y_step = dict.get::<f32>(Y_STEP)?;

        if x_step.is_nearly_zero() || y_step.is_nearly_zero() || bbox.is_zero_area() {
            return None;
        }

        let is_color = dict.get::<u8>(PAINT_TYPE)? == 1;
        let matrix = dict
            .get::<[f64; 6]>(MATRIX)
            .map(Affine::new)
            .unwrap_or_default();

        let state = ctx.get().clone();
        let ctx_bbox = ctx.bbox();

        let fill_cs = state
            .graphics_state
            .none_stroke_cs
            .pattern_cs()
            .unwrap_or(ColorSpace::device_gray());
        let stroke_cs = state
            .graphics_state
            .stroke_cs
            .pattern_cs()
            .unwrap_or(ColorSpace::device_gray());

        let non_stroking_paint = Color::new(
            fill_cs,
            state.graphics_state.non_stroke_color.clone(),
            state.graphics_state.non_stroke_alpha,
        );
        let stroke_paint = Color::new(
            stroke_cs,
            state.graphics_state.stroke_color.clone(),
            state.graphics_state.stroke_alpha,
        );

        Some(Self {
            cache_key,
            bbox,
            x_step,
            y_step,
            matrix,
            ctx_bbox,
            is_color,
            stream,
            stroke_paint,
            non_stroking_paint,
            state: Box::new(ctx.get().clone()),
            settings: ctx.settings.clone(),
            parent_resources: resources.clone(),
            cache: ctx.object_cache.clone(),
            xref: ctx.xref,
        })
    }

    /// Interpret the contents of the pattern into the given device.
    pub fn interpret(
        &self,
        device: &mut impl Device<'a>,
        initial_transform: Affine,
        is_stroke: bool,
    ) -> Option<()> {
        let mut state = (*self.state).clone();
        state.ctm = initial_transform;
        // Not sure if this is mentioned anywhere, but I do think we need to reset the text state
        // (though the graphics state itself should be preserved).
        state.text_state = TextState::default();

        let mut context = Context::new_with(
            state.ctm,
            // TODO: bbox?
            (initial_transform * self.ctx_bbox.to_path(0.1)).bounding_box(),
            self.cache.clone(),
            self.xref,
            self.settings.clone(),
            state,
        );

        let decoded = self.stream.decoded().ok()?;
        let resources = Resources::from_parent(
            self.stream.dict().get(RESOURCES).unwrap_or_default(),
            self.parent_resources.clone(),
        );
        let iter = TypedIter::new(decoded.as_ref());

        let clip_path = ClipPath {
            path: initial_transform * self.bbox.to_path(0.1),
            fill: FillRule::NonZero,
        };
        device.push_clip_path(&clip_path);

        if self.is_color {
            interpret(iter, &resources, &mut context, device);
        } else {
            let paint = if !is_stroke {
                Paint::Color(self.non_stroking_paint.clone())
            } else {
                Paint::Color(self.stroke_paint.clone())
            };

            let mut device = StencilPatternDevice::new(device, paint.clone());
            interpret(iter, &resources, &mut context, &mut device);
        }

        device.pop_clip_path();

        Some(())
    }
}

impl CacheKey for TilingPattern<'_> {
    fn cache_key(&self) -> u128 {
        self.cache_key
    }
}

struct StencilPatternDevice<'a, 'b, T: Device<'a>> {
    inner: &'b mut T,
    paint: Paint<'a>,
}

impl<'a, 'b, T: Device<'a>> StencilPatternDevice<'a, 'b, T> {
    pub fn new(device: &'b mut T, paint: Paint<'a>) -> Self {
        Self {
            inner: device,
            paint,
        }
    }
}

// Only filling, stroking of paths and stencil masks are allowed.
impl<'a, T: Device<'a>> Device<'a> for StencilPatternDevice<'a, '_, T> {
    fn draw_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        _: &Paint,
        draw_mode: &PathDrawMode,
    ) {
        self.inner
            .draw_path(path, transform, &self.paint, draw_mode)
    }

    fn set_soft_mask(&mut self, _: Option<SoftMask>) {}

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.inner.push_clip_path(clip_path)
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask>, _: BlendMode) {}

    fn draw_glyph(
        &mut self,
        g: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        p: &Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        self.inner
            .draw_glyph(g, transform, glyph_transform, p, draw_mode);
    }

    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
        if let Image::Stencil(mut s) = image {
            s.paint = self.paint.clone();
            self.inner.draw_image(Image::Stencil(s), transform)
        }
    }

    fn pop_clip_path(&mut self) {
        self.inner.pop_clip_path();
    }

    fn pop_transparency_group(&mut self) {}

    fn set_blend_mode(&mut self, _: BlendMode) {}
}
