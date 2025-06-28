use crate::clip_path::ClipPath;
use crate::context::Context;
use crate::device::Device;
use crate::font::glyph_simulator::GlyphSimulator;
use crate::font::true_type::{read_encoding, read_widths};
use crate::font::{Encoding, Glyph, Type3Glyph, UNITS_PER_EM};
use crate::image::{RgbaImage, StencilImage};
use crate::paint::Paint;
use crate::{FillProps, StrokeProps, interpret};
use hayro_syntax::content::ops::TypedOperation;
use hayro_syntax::content::{TypedIter, UntypedIter};
use hayro_syntax::document::page::Resources;
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::dict::keys::{CHAR_PROCS, FONT_MATRIX, RESOURCES};
use hayro_syntax::object::stream::Stream;
use kurbo::{Affine, BezPath, Rect};
use skrifa::GlyphId;
use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct Type3<'a> {
    widths: Vec<f32>,
    encoding: Encoding,
    encodings: HashMap<u8, String>,
    dict: Dict<'a>,
    char_procs: HashMap<String, Stream<'a>>,
    glyph_simulator: GlyphSimulator,
    matrix: Affine,
}

impl<'a> Type3<'a> {
    pub(crate) fn new(dict: &Dict<'a>) -> Self {
        let (encoding, encodings) = read_encoding(dict);
        let widths = read_widths(dict, dict);

        let matrix = Affine::new(
            dict.get::<[f64; 6]>(FONT_MATRIX)
                .unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]),
        );

        let char_procs = {
            let mut procs = HashMap::new();
            let dict = dict.get::<Dict>(CHAR_PROCS).unwrap_or_default();

            for name in dict.keys() {
                let prog = dict.get::<Stream>(name.clone()).unwrap();

                procs.insert(name.as_str().to_string(), prog.clone());
            }

            procs
        };

        Self {
            glyph_simulator: GlyphSimulator::new(),
            encoding,
            char_procs,
            widths,
            encodings,
            matrix,
            dict: dict.clone(),
        }
    }

    pub(crate) fn map_code(&self, code: u8) -> GlyphId {
        self.encodings
            .get(&code)
            .map(|s| s.as_str())
            .or_else(|| self.encoding.map_code(code))
            .map(|g| self.glyph_simulator.string_to_glyph(g))
            .unwrap_or(GlyphId::NOTDEF)
    }

    pub(crate) fn glyph_width(&self, code: u8) -> f32 {
        (*self.widths.get(code as usize).unwrap() * self.matrix.as_coeffs()[0] as f32)
            * UNITS_PER_EM
    }

    pub(crate) fn render_glyph(
        &self,
        glyph: &Type3Glyph,
        paint: &Paint,
        device: &mut impl Device,
    ) -> Option<()> {
        let mut state = glyph.state.clone();
        let root_transform =
            state.ctm * glyph.glyph_transform * self.matrix * Affine::scale(UNITS_PER_EM as f64);
        state.ctm = root_transform;

        let mut context = Context::new_with(
            state.ctm,
            // TODO: bbox?
            Rect::new(0.0, 0.0, 1.0, 1.0),
            glyph.cache.clone(),
            glyph.xref,
            state,
        );

        let name = self.glyph_simulator.glyph_to_string(glyph.glyph_id)?;
        let program = self.char_procs.get(&name)?;
        let decoded = program.decoded()?;
        let iter = TypedIter::new(UntypedIter::new(decoded.as_ref()));

        let is_shape_glyph = {
            let iter = iter.clone();
            let mut is_shape_glyph = true;

            for op in iter {
                match op {
                    TypedOperation::ShapeGlyph(_) => {
                        break;
                    }
                    TypedOperation::ColorGlyph(_) => {
                        is_shape_glyph = false;
                        break;
                    }
                    _ => {}
                }
            }

            is_shape_glyph
        };

        let resources = Resources::from_parent(
            self.dict.get(RESOURCES).unwrap_or_default(),
            glyph.parent_resources.clone(),
        );

        if is_shape_glyph {
            let mut device = Type3ShapeGlyphDevice::new(device, paint);
            interpret(iter, &resources, &mut context, &mut device);
        } else {
            interpret(iter, &resources, &mut context, device);
        }

        Some(())
    }
}

struct Type3ShapeGlyphDevice<'a, T: Device> {
    inner: &'a mut T,
    paint: &'a Paint<'a>,
}

impl<'a, T: Device> Type3ShapeGlyphDevice<'a, T> {
    pub fn new(device: &'a mut T, paint: &'a Paint<'a>) -> Self {
        Self {
            inner: device,
            paint,
        }
    }
}

// Only filling, stroking of paths and stencil masks are allowed.
impl<T: Device> Device for Type3ShapeGlyphDevice<'_, T> {
    fn set_transform(&mut self, affine: Affine) {
        self.inner.set_transform(affine);
    }

    fn stroke_path(&mut self, path: &BezPath, _: &Paint) {
        self.inner.stroke_path(path, self.paint)
    }

    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps) {
        self.inner.set_stroke_properties(stroke_props)
    }

    fn fill_path(&mut self, path: &BezPath, _: &Paint) {
        self.inner.fill_path(path, self.paint)
    }

    fn set_fill_properties(&mut self, fill_props: &FillProps) {
        self.inner.set_fill_properties(fill_props)
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.inner.push_clip_path(clip_path)
    }

    fn push_transparency_group(&mut self, _: f32) {}

    fn fill_glyph(&mut self, _: &Glyph<'_>, _: &Paint) {}

    fn stroke_glyph(&mut self, _: &Glyph<'_>, _: &Paint) {}

    fn draw_rgba_image(&mut self, _: RgbaImage) {}

    fn draw_stencil_image(&mut self, stencil: StencilImage, _: &Paint) {
        self.inner.draw_stencil_image(stencil, self.paint);
    }

    fn pop_clip_path(&mut self) {
        self.inner.pop_clip_path()
    }

    fn pop_transparency_group(&mut self) {}
}
