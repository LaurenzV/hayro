use crate::cache::Cache;
use crate::clip_path::ClipPath;
use crate::color::{Color, ColorSpace};
use crate::context::Context;
use crate::device::Device;
use crate::font::Glyph;
use crate::interpret::state::State;
use crate::shading::Shading;
use crate::soft_mask::SoftMask;
use crate::{
    AlphaData, FillProps, FillRule, InterpreterSettings, Paint, PaintType, RgbData, StrokeProps,
    interpret,
};
use hayro_syntax::content::{TypedIter, UntypedIter};
use hayro_syntax::document::page::Resources;
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::dict::keys::{
    BBOX, EXT_G_STATE, MATRIX, PAINT_TYPE, RESOURCES, SHADING, X_STEP, Y_STEP,
};
use hayro_syntax::object::rect::Rect;
use hayro_syntax::object::stream::Stream;
use hayro_syntax::object::{Object, dict_or_stream};
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, Shape};
use log::warn;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum Pattern<'a> {
    Shading(ShadingPattern),
    Tiling(TilingPattern<'a>),
}

impl<'a> Pattern<'a> {
    pub fn new(object: Object<'a>, ctx: &Context<'a>, resources: &Resources<'a>) -> Option<Self> {
        if let Some(dict) = object.clone().into_dict() {
            Some(Self::Shading(ShadingPattern::new(&dict)?))
        } else if let Some(stream) = object.clone().into_stream() {
            Some(Self::Tiling(TilingPattern::new(stream, ctx, resources)?))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct ShadingPattern {
    pub shading: Arc<Shading>,
    pub matrix: Affine,
}

impl ShadingPattern {
    pub fn new(dict: &Dict) -> Option<Self> {
        let shading = dict.get::<Object>(SHADING).and_then(|o| {
            let (dict, stream) = dict_or_stream(&o)?;

            Shading::new(&dict, stream.as_ref())
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

#[derive(Clone)]
pub struct TilingPattern<'a> {
    pub bbox: Rect,
    pub x_step: f32,
    pub y_step: f32,
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
    pub fn new(stream: Stream<'a>, ctx: &Context<'a>, resources: &Resources<'a>) -> Option<Self> {
        let dict = stream.dict();

        let bbox = dict.get::<Rect>(BBOX)?;
        let x_step = dict.get::<f32>(X_STEP)?;
        let y_step = dict.get::<f32>(Y_STEP)?;
        let is_color = dict.get::<u8>(PAINT_TYPE)? == 1;
        let matrix = dict
            .get::<[f64; 6]>(MATRIX)
            .map(Affine::new)
            .unwrap_or_default();

        let state = ctx.get();

        let fill_cs = ctx
            .get()
            .none_stroke_cs
            .pattern_cs()
            .unwrap_or(ColorSpace::device_gray());
        let stroke_cs = ctx
            .get()
            .stroke_cs
            .pattern_cs()
            .unwrap_or(ColorSpace::device_gray());

        let non_stroking_paint = Color::new(
            fill_cs,
            state.non_stroke_color.clone(),
            state.non_stroke_alpha,
        );
        let stroke_paint = Color::new(stroke_cs, state.stroke_color.clone(), state.stroke_alpha);

        Some(Self {
            bbox,
            x_step,
            y_step,
            matrix,
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

    pub fn interpret(
        &self,
        device: &mut impl Device,
        initial_transform: Affine,
        is_stroke: bool,
    ) -> Option<()> {
        let mut state = (*self.state).clone();
        state.ctm = initial_transform;

        let mut context = Context::new_with(
            state.ctm,
            // TODO: bbox?
            kurbo::Rect::new(0.0, 0.0, 1.0, 1.0),
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
        let iter = TypedIter::new(UntypedIter::new(decoded.as_ref()));

        let clip_path = ClipPath {
            path: initial_transform * self.bbox.to_path(0.1),
            fill: FillRule::NonZero,
        };
        device.push_clip_path(&clip_path);

        if self.is_color {
            interpret(iter, &resources, &mut context, device);
        } else {
            let paint = if !is_stroke {
                Paint {
                    paint_transform: Default::default(),
                    paint_type: PaintType::Color(self.non_stroking_paint.clone()),
                }
            } else {
                Paint {
                    paint_transform: Default::default(),
                    paint_type: PaintType::Color(self.stroke_paint.clone()),
                }
            };

            let mut device = StencilPatternDevice::new(device, &paint);
            interpret(iter, &resources, &mut context, &mut device);
        }

        device.pop_clip_path();

        Some(())
    }
}

struct StencilPatternDevice<'a, T: Device> {
    inner: &'a mut T,
    paint: &'a Paint<'a>,
}

impl<'a, T: Device> StencilPatternDevice<'a, T> {
    pub fn new(device: &'a mut T, paint: &'a Paint<'a>) -> Self {
        Self {
            inner: device,
            paint,
        }
    }
}

// Only filling, stroking of paths and stencil masks are allowed.
impl<T: Device> Device for StencilPatternDevice<'_, T> {
    fn set_transform(&mut self, affine: Affine) {
        self.inner.set_transform(affine);
    }

    fn stroke_path(&mut self, path: &BezPath, _: &Paint) {
        self.inner.stroke_path(path, self.paint)
    }

    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps) {
        self.inner.set_stroke_properties(stroke_props)
    }

    fn set_soft_mask(&mut self, _: Option<SoftMask>) {}

    fn fill_path(&mut self, path: &BezPath, _: &Paint) {
        self.inner.fill_path(path, self.paint)
    }

    fn set_fill_properties(&mut self, fill_props: &FillProps) {
        self.inner.set_fill_properties(fill_props)
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.inner.push_clip_path(clip_path)
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask>) {}

    fn fill_glyph(&mut self, glyph: &Glyph<'_>, _: &Paint) {
        self.inner.fill_glyph(glyph, self.paint)
    }

    fn stroke_glyph(&mut self, glyph: &Glyph<'_>, _: &Paint) {
        self.inner.stroke_glyph(glyph, self.paint)
    }

    fn draw_rgba_image(&mut self, _: RgbData, _: Option<AlphaData>) {}

    fn draw_stencil_image(&mut self, stencil: AlphaData, _: &Paint) {
        self.inner.draw_stencil_image(stencil, self.paint);
    }

    fn pop_clip_path(&mut self) {
        self.inner.pop_clip_path();
    }

    fn pop_transparency_group(&mut self) {}
}
