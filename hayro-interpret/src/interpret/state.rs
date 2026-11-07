use crate::StrokeProps;
use crate::color::{AlphaColor, ColorComponents, ColorSpace};
use crate::context::Context;
use crate::convert::{convert_line_cap, convert_line_join};
use crate::font::{Font, UNITS_PER_EM};
use crate::function::{Function, TransferFunction};
use crate::interpret::text::TextRenderingMode;
use crate::pattern::Pattern;
use crate::types::BlendMode;
use crate::x_object::soft_mask::SoftMask;
use hayro_syntax::content::ops::{LineCap, LineJoin};
use hayro_syntax::object::dict::keys::{FONT, SMASK, TR, TR2};
use hayro_syntax::object::{Array, Dict, Name, Number, Object};
use hayro_syntax::page::Resources;
use kurbo::{Affine, BezPath, Vec2};
use smallvec::smallvec;
use std::ops::Deref;

/// A transfer function.
#[derive(Clone, Debug)]
pub enum ActiveTransferFunction {
    /// A single transfer function applied to all components.
    Single(TransferFunction),
    /// Four transfer functions, one for each component.
    Four([TransferFunction; 4]),
}

impl ActiveTransferFunction {
    /// Apply the transfer function to the RGB channels of an RGBA color.
    /// The alpha channel is left unchanged.
    pub fn apply(&self, color: &AlphaColor) -> AlphaColor {
        let mut rgba = color.components();

        match self {
            Self::Single(f) => {
                for c in &mut rgba[..3] {
                    *c = f.apply_f32(*c);
                }
            }
            Self::Four(functions) => {
                for (i, f) in functions[..3].iter().enumerate() {
                    rgba[i] = f.apply_f32(rgba[i]);
                }
            }
        }

        AlphaColor::new(rgba)
    }

    pub(crate) fn apply_to(&self, values: &mut [u8]) {
        match self {
            Self::Single(function) => function.apply_to(values),
            Self::Four(functions) => {
                // Since we are converting to RGB, we only need the first three.
                for (channel, function) in functions[..3].iter().enumerate() {
                    function.apply_to_stride(&mut values[channel..], 3);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ClipType {
    Dummy,
    Real,
}

/// The graphics state.
#[derive(Clone, Debug)]
pub(crate) struct State<'a> {
    pub(crate) ctm: Affine,
    pub(crate) stroke: PaintState<'a>,
    pub(crate) fill: PaintState<'a>,
    pub(crate) stroke_props: StrokeProps,
    pub(crate) text_state: TextState<'a>,
    pub(crate) compositing: CompositingState<'a>,
}

impl Default for State<'_> {
    fn default() -> Self {
        State {
            ctm: Affine::IDENTITY,
            stroke: PaintState::default(),
            fill: PaintState::default(),
            stroke_props: StrokeProps::default(),
            text_state: TextState::default(),
            compositing: CompositingState::default(),
        }
    }
}

impl<'a> State<'a> {
    pub(crate) fn new(initial_transform: Affine) -> Self {
        Self {
            ctm: initial_transform,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum TextStateFont<'a> {
    /// The font in the text state was explicitly set and resolves to a valid
    /// font.
    Font(Font<'a>),
    /// The font was not set or an invalid font was set.
    Fallback(Font<'a>),
}

impl<'a> Deref for TextStateFont<'a> {
    type Target = Font<'a>;

    fn deref(&self) -> &Self::Target {
        match self {
            TextStateFont::Font(f) => f,
            TextStateFont::Fallback(f) => f,
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
    pub(crate) font: Option<TextStateFont<'a>>,
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

    pub(crate) fn font_horizontal(&self) -> bool {
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
            render_mode: TextRenderingMode::default(),
            text_matrix: Affine::IDENTITY,
            text_line_matrix: Affine::IDENTITY,
            rise: 0.0,
            clip_paths: BezPath::default(),
        }
    }
}

/// Paint parameters for strokes or fills.
#[derive(Clone, Debug)]
pub(crate) struct PaintState<'a> {
    pub(crate) color: ColorComponents,
    pub(crate) pattern: Option<Pattern<'a>>,
    pub(crate) color_space: ColorSpace,
    pub(crate) alpha: f32,
}

impl Default for PaintState<'_> {
    fn default() -> Self {
        Self {
            color: smallvec![0.0],
            pattern: None,
            color_space: ColorSpace::device_gray(),
            alpha: 1.0,
        }
    }
}

/// Compositing parameters.
#[derive(Clone, Debug, Default)]
pub(crate) struct CompositingState<'a> {
    pub(crate) soft_mask: Option<SoftMask<'a>>,
    pub(crate) transfer_function: Option<ActiveTransferFunction>,
    pub(crate) blend_mode: BlendMode,
}

pub(crate) struct PaintData<'a> {
    pub(crate) alpha: f32,
    pub(crate) color: ColorComponents,
    pub(crate) color_space: ColorSpace,
    pub(crate) pattern: Option<Pattern<'a>>,
    pub(crate) transfer_function: Option<ActiveTransferFunction>,
}

pub(crate) fn handle_gs<'a>(
    dict: &Dict<'a>,
    context: &mut Context<'a>,
    parent_resources: &Resources<'a>,
) {
    for key in dict.keys() {
        handle_gs_single(dict, key.clone(), context, parent_resources);
    }
}

pub(crate) fn handle_gs_single<'a>(
    dict: &Dict<'a>,
    key: Name<'_>,
    context: &mut Context<'a>,
    parent_resources: &Resources<'a>,
) -> Option<()> {
    // TODO Can we use constants here somehow?
    match key.as_str() {
        "LW" => context.stroke_props_mut().line_width = dict.get::<f32>(key)?,
        "LC" => {
            context.stroke_props_mut().line_cap =
                convert_line_cap(LineCap(dict.get::<Number>(key)?));
        }
        "LJ" => {
            context.stroke_props_mut().line_join =
                convert_line_join(LineJoin(dict.get::<Number>(key)?));
        }
        "ML" => context.stroke_props_mut().miter_limit = dict.get::<f32>(key)?,
        "CA" => context.stroke_paint_mut().alpha = dict.get::<f32>(key)?,
        "ca" => context.fill_paint_mut().alpha = dict.get::<f32>(key)?,
        "TR" | "TR2" => {
            let function = match dict
                .get::<Object<'_>>(TR2)
                .or_else(|| dict.get::<Object<'_>>(TR))?
            {
                Object::Array(array) => {
                    let mut iter = array.iter::<Object<'_>>();
                    let functions = [
                        TransferFunction::new(Function::new(&iter.next()?)?),
                        TransferFunction::new(Function::new(&iter.next()?)?),
                        TransferFunction::new(Function::new(&iter.next()?)?),
                        TransferFunction::new(Function::new(&iter.next()?)?),
                    ];

                    Some(ActiveTransferFunction::Four(functions))
                }
                // Only `Identity` and `Default` are valid, which both just reset it.
                Object::Name(_) => None,
                o => Some(ActiveTransferFunction::Single(TransferFunction::new(
                    Function::new(&o)?,
                ))),
            };

            context.compositing_mut().transfer_function = function;
        }
        "SMask" => {
            if let Some(name) = dict.get::<Name<'_>>(SMASK) {
                if name.deref() == b"None" {
                    context.compositing_mut().soft_mask = None;
                }
            } else {
                context.compositing_mut().soft_mask = dict
                    .get::<Dict<'_>>(SMASK)
                    .and_then(|d| SoftMask::new(&d, context, parent_resources.clone()));
            }
        }
        "BM" => {
            if let Some(name) = dict.get::<Name<'_>>(key.as_ref()) {
                if let Some(bm) = convert_blend_mode(name.as_str()) {
                    context.compositing_mut().blend_mode = bm;

                    return Some(());
                }
            } else if let Some(arr) = dict.get::<Array<'_>>(key) {
                for name in arr.iter::<Name<'_>>() {
                    if let Some(bm) = convert_blend_mode(name.as_str()) {
                        context.compositing_mut().blend_mode = bm;

                        return Some(());
                    }
                }
            }

            context.compositing_mut().blend_mode = BlendMode::Normal;
        }
        "Font" => {
            let arr = dict.get::<Array<'_>>(FONT)?;
            let mut iter = arr.iter::<Object<'_>>();
            let font_dict = iter.next()?.into_dict()?;
            let size = iter.next()?.into_number()?.as_f32();

            let font = context.resolve_font(&font_dict);
            context.text_state_mut().font_size = size;
            context.text_state_mut().font = font;
        }
        "D" => {
            let arr = dict.get::<Array<'_>>(key)?;
            let mut iter = arr.iter::<Object<'_>>();
            let dash_array = iter.next()?.into_array()?;
            let dash_phase = iter.next()?.into_number()?.as_f32();

            context.stroke_props_mut().dash_offset = dash_phase;
            context.stroke_props_mut().dash_array = dash_array
                .iter::<f32>()
                // kurbo apparently cannot properly deal with offsets that are exactly 0.
                .map(|n| if n == 0.0 { 0.01 } else { n })
                .collect();
        }
        "Type" => {}
        _ => {}
    }

    Some(())
}

fn convert_blend_mode(name: &str) -> Option<BlendMode> {
    let bm = match name {
        "Normal" => BlendMode::Normal,
        "Multiply" => BlendMode::Multiply,
        "Screen" => BlendMode::Screen,
        "Overlay" => BlendMode::Overlay,
        "Darken" => BlendMode::Darken,
        "Lighten" => BlendMode::Lighten,
        "ColorDodge" => BlendMode::ColorDodge,
        "ColorBurn" => BlendMode::ColorBurn,
        "HardLight" => BlendMode::HardLight,
        "SoftLight" => BlendMode::SoftLight,
        "Difference" => BlendMode::Difference,
        "Exclusion" => BlendMode::Exclusion,
        "Hue" => BlendMode::Hue,
        "Saturation" => BlendMode::Saturation,
        "Color" => BlendMode::Color,
        "Luminosity" => BlendMode::Luminosity,
        _ => return None,
    };

    Some(bm)
}
