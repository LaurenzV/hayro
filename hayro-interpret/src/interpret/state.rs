use crate::StrokeProps;
use crate::color::{ColorComponents, ColorSpace};
use crate::context::Context;
use crate::convert::{convert_line_cap, convert_line_join};
use crate::device::Device;
use crate::font::{Font, UNITS_PER_EM};
use crate::interpret::text::TextRenderingMode;
use crate::pattern::Pattern;
use crate::soft_mask::SoftMask;
use crate::util::OptionLog;
use hayro_syntax::content::ops::{LineCap, LineJoin};
use hayro_syntax::object::dict::keys::SMASK;
use hayro_syntax::object::{Dict, Name, Number};
use hayro_syntax::page::Resources;
use kurbo::{Affine, BezPath, Vec2};
use log::warn;
use smallvec::smallvec;
use std::ops::Deref;

#[derive(Clone, Debug)]
pub(crate) struct State<'a> {
    // Note that the text state and ctm are theoretically part of the graphics state,
    // but we keep them separate for simplicity.
    pub(crate) graphics_state: GraphicsState<'a>,
    pub(crate) text_state: TextState<'a>,
    pub(crate) ctm: Affine,
    // Strictly speaking not part of the graphics state, but we keep it there for
    // consistency.
    pub(crate) n_clips: u32,
}

impl Default for State<'_> {
    fn default() -> Self {
        State {
            ctm: Affine::IDENTITY,
            n_clips: 0,
            text_state: TextState::default(),
            graphics_state: GraphicsState::default(),
        }
    }
}

impl<'a> State<'a> {
    pub(crate) fn new(initial_transform: Affine) -> Self {
        Self {
            ctm: initial_transform,
            ..Default::default()
        }
    }

    pub(crate) fn stroke_data(&self) -> PaintData<'a> {
        PaintData {
            alpha: self.graphics_state.stroke_alpha,
            color: self.graphics_state.stroke_color.clone(),
            color_space: self.graphics_state.stroke_cs.clone(),
            pattern: self.graphics_state.stroke_pattern.clone(),
        }
    }

    pub(crate) fn non_stroke_data(&self) -> PaintData<'a> {
        PaintData {
            alpha: self.graphics_state.non_stroke_alpha,
            color: self.graphics_state.non_stroke_color.clone(),
            color_space: self.graphics_state.none_stroke_cs.clone(),
            pattern: self.graphics_state.non_stroke_pattern.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TextState<'a> {
    pub(crate) char_space: f32,
    pub(crate) word_space: f32,
    // Note that this stores 1/100 of the actual scaling.
    pub(crate) horizontal_scaling: f32,
    pub(crate) leading: f32,
    pub(crate) font: Option<Font<'a>>,
    pub(crate) font_size: f32,
    pub(crate) rise: f32,
    pub(crate) render_mode: TextRenderingMode,

    pub(crate) text_matrix: Affine,
    pub(crate) text_line_matrix: Affine,

    // When setting the text rendering mode to `clip`, the glyphs should instead be collected
    // as paths and then applied as 1 single clip path. This field stores those clip paths.
    pub(crate) clip_paths: BezPath,
}

impl<'a> TextState<'a> {
    fn temp_transform(&self) -> Affine {
        Affine::new([
            self.font_size as f64 * self.horizontal_scaling() as f64,
            0.0,
            0.0,
            self.font_size as f64,
            0.0,
            self.rise as f64,
        ])
    }

    fn horizontal_scaling(&self) -> f32 {
        self.horizontal_scaling / 100.0
    }

    fn font_horizontal(&self) -> bool {
        self.font
            .as_ref()
            .map(|f| f.is_horizontal())
            .unwrap_or(false)
    }

    pub(crate) fn apply_adjustment(&mut self, adjustment: f32) {
        let horizontal = self.font_horizontal();

        let horizontal_scaling = if horizontal {
            self.horizontal_scaling()
        } else {
            1.0
        };

        let scaled_adjustment = -adjustment / UNITS_PER_EM * self.font_size * horizontal_scaling;
        let (tx, ty) = if horizontal {
            (scaled_adjustment, 0.0)
        } else {
            (0.0, scaled_adjustment)
        };

        self.text_matrix *= Affine::new([1.0, 0.0, 0.0, 1.0, tx as f64, ty as f64]);
    }

    pub(crate) fn apply_code_advance(&mut self, char_code: u32, code_len: usize) {
        let glyph_advance = self
            .font
            .as_ref()
            .map(|f| f.code_advance(char_code))
            .unwrap_or(Vec2::ZERO);
        let horizontal = self.font_horizontal();

        let word_space = if char_code == 32 && code_len == 1 {
            self.word_space
        } else {
            0.0
        };

        let base_advance =
            |advance: f32| advance / UNITS_PER_EM * self.font_size + self.char_space + word_space;

        let tx = if horizontal {
            base_advance(glyph_advance.x as f32) * self.horizontal_scaling()
        } else {
            0.0
        };

        let ty = if !horizontal {
            base_advance(glyph_advance.y as f32)
        } else {
            0.0
        };

        self.text_matrix *= Affine::new([1.0, 0.0, 0.0, 1.0, tx as f64, ty as f64]);
    }

    pub(crate) fn full_transform(&self) -> Affine {
        self.text_matrix * self.temp_transform()
    }
}

impl Default for TextState<'_> {
    fn default() -> Self {
        Self {
            char_space: 0.0,
            word_space: 0.0,
            horizontal_scaling: 100.0,
            leading: 0.0,
            font: None,
            // Not in the specification, but we just define it so we don't need to use an option.
            font_size: 1.0,
            render_mode: Default::default(),
            text_matrix: Affine::IDENTITY,
            text_line_matrix: Affine::IDENTITY,
            rise: 0.0,
            clip_paths: BezPath::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GraphicsState<'a> {
    // Stroke parameters.
    pub(crate) stroke_props: StrokeProps,

    // Stroke paint parameters.
    pub(crate) stroke_color: ColorComponents,
    pub(crate) stroke_pattern: Option<Pattern<'a>>,
    pub(crate) stroke_cs: ColorSpace,
    pub(crate) stroke_alpha: f32,

    // Non-stroke paint parameters.
    pub(crate) non_stroke_color: ColorComponents,
    pub(crate) non_stroke_pattern: Option<Pattern<'a>>,
    pub(crate) none_stroke_cs: ColorSpace,
    pub(crate) non_stroke_alpha: f32,

    pub(crate) soft_mask: Option<SoftMask<'a>>,
}

impl Default for GraphicsState<'_> {
    fn default() -> Self {
        GraphicsState {
            stroke_props: StrokeProps::default(),
            non_stroke_alpha: 1.0,
            stroke_cs: ColorSpace::device_gray(),
            stroke_color: smallvec![0.0,],
            none_stroke_cs: ColorSpace::device_gray(),
            non_stroke_color: smallvec![0.0],
            stroke_alpha: 1.0,
            stroke_pattern: None,
            non_stroke_pattern: None,
            soft_mask: None,
        }
    }
}

pub(crate) struct PaintData<'a> {
    pub(crate) alpha: f32,
    pub(crate) color: ColorComponents,
    pub(crate) color_space: ColorSpace,
    pub(crate) pattern: Option<Pattern<'a>>,
}

pub(crate) fn save_sate(ctx: &mut Context) {
    ctx.save_state();
}

pub(crate) fn restore_state<'a>(ctx: &mut Context<'a>, device: &mut impl Device<'a>) {
    let mut num_clips = ctx.get().n_clips;
    ctx.restore_state();
    let target_clips = ctx.get().n_clips;

    while num_clips > target_clips {
        device.pop_clip_path();
        ctx.pop_bbox();
        num_clips -= 1;
    }
}

pub(crate) fn handle_gs<'a>(
    dict: &Dict<'a>,
    context: &mut Context<'a>,
    parent_resources: &Resources<'a>,
) {
    for key in dict.keys() {
        handle_gs_single(dict, key.clone(), context, parent_resources).warn_none(&format!(
            "invalid value in graphics state for {}",
            key.as_str()
        ));
    }
}

pub(crate) fn handle_gs_single<'a>(
    dict: &Dict<'a>,
    key: Name,
    context: &mut Context<'a>,
    parent_resources: &Resources<'a>,
) -> Option<()> {
    // TODO Can we use constants here somehow?
    match key.as_str() {
        "LW" => context.get_mut().graphics_state.stroke_props.line_width = dict.get::<f32>(key)?,
        "LC" => {
            context.get_mut().graphics_state.stroke_props.line_cap =
                convert_line_cap(LineCap(dict.get::<Number>(key)?))
        }
        "LJ" => {
            context.get_mut().graphics_state.stroke_props.line_join =
                convert_line_join(LineJoin(dict.get::<Number>(key)?))
        }
        "ML" => context.get_mut().graphics_state.stroke_props.miter_limit = dict.get::<f32>(key)?,
        "CA" => context.get_mut().graphics_state.stroke_alpha = dict.get::<f32>(key)?,
        "ca" => context.get_mut().graphics_state.non_stroke_alpha = dict.get::<f32>(key)?,
        "SMask" => {
            if let Some(name) = dict.get::<Name>(SMASK) {
                if name.deref() == b"None" {
                    context.get_mut().graphics_state.soft_mask = None;
                }
            } else {
                context.get_mut().graphics_state.soft_mask = dict
                    .get::<Dict>(SMASK)
                    .and_then(|d| SoftMask::new(&d, context, parent_resources.clone()));
            }
        }
        "BM" => {
            let name = dict.get::<Name>(key)?;
            let mode = name.as_str();
            if mode != "Normal" {
                warn!("blend mode {mode} is not supported");
            }
        }
        "Type" => {}
        _ => {}
    }

    Some(())
}
