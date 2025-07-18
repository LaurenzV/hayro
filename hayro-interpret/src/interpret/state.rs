use crate::FillRule;
use crate::color::{ColorComponents, ColorSpace};
use crate::font::{Font, UNITS_PER_EM};
use crate::interpret::text::TextRenderingMode;
use crate::pattern::Pattern;
use crate::soft_mask::SoftMask;
use kurbo::{Affine, BezPath, Cap, Join, Vec2};
use smallvec::SmallVec;

#[derive(Clone, Debug)]
pub(crate) struct State<'a> {
    // Stroke parameters.
    pub(crate) line_width: f32,
    pub(crate) line_cap: Cap,
    pub(crate) line_join: Join,
    pub(crate) miter_limit: f32,
    pub(crate) dash_array: SmallVec<[f32; 4]>,
    pub(crate) dash_offset: f32,

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

    // Text state.
    pub(crate) text_state: TextState<'a>,

    // CTM.
    pub(crate) ctm: Affine,

    // Miscellaneous.
    pub(crate) soft_mask: Option<SoftMask<'a>>,
    // Strictly speaking not part of the graphics state, but we keep it there for
    // consistency.
    pub(crate) fill_rule: FillRule,
    pub(crate) n_clips: u32,
}

impl<'a> State<'a> {
    pub(crate) fn stroke_data(&self) -> PaintData<'a> {
        PaintData {
            alpha: self.stroke_alpha,
            color: self.stroke_color.clone(),
            color_space: self.stroke_cs.clone(),
            pattern: self.stroke_pattern.clone(),
        }
    }

    pub(crate) fn non_stroke_data(&self) -> PaintData<'a> {
        PaintData {
            alpha: self.non_stroke_alpha,
            color: self.non_stroke_color.clone(),
            color_space: self.none_stroke_cs.clone(),
            pattern: self.non_stroke_pattern.clone(),
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

    pub(crate) fn apply_code_advance(&mut self, char_code: u16) {
        let code_len = self.font.as_ref().map(|f| f.code_len()).unwrap_or(1);
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

pub(crate) struct PaintData<'a> {
    pub(crate) alpha: f32,
    pub(crate) color: ColorComponents,
    pub(crate) color_space: ColorSpace,
    pub(crate) pattern: Option<Pattern<'a>>,
}
