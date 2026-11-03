use crate::context::Context;
use crate::device::Device;
use crate::font::{GlyphRun, Type3Glyph, UNITS_PER_EM};
use crate::interpret::state::TextState;
use crate::x_object::soft_mask::SoftMask;
use crate::{BlendMode, interpret};
use crate::{ClipPath, DrawMode, DrawProps, ImageDrawProps};
use crate::{Image, Paint};
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::RESOURCES;
use hayro_syntax::page::Resources;
use kurbo::{Affine, BezPath};

pub(crate) fn render_glyph<'a>(
    glyph: &Type3Glyph<'a>,
    transform: Affine,
    glyph_transform: Affine,
    paint: &Paint<'a>,
    device: &mut impl Device<'a>,
) -> Option<()> {
    let mut state = glyph.state.clone();
    let root_transform = transform
        * glyph_transform
        * glyph.glyph.font().matrix()
        * Affine::scale(UNITS_PER_EM as f64);
    state.ctm = root_transform;

    // Not sure if this is mentioned anywhere, but I do think we need to reset the text state
    // (though the graphics state itself should be preserved).
    state.text_state = TextState::default();

    let program = glyph.glyph.program()?;
    let decoded = program.decoded_data().ok()?;
    let iter = TypedIter::new(decoded.as_ref());

    let is_shape_glyph = {
        let mut iter = iter.clone();
        let mut is_shape_glyph = true;

        while let Some(op) = iter.next() {
            match op {
                TypedInstruction::ShapeGlyph(_) => {
                    break;
                }
                TypedInstruction::ColorGlyph(_) => {
                    is_shape_glyph = false;
                    break;
                }
                _ => {}
            }
        }

        is_shape_glyph
    };

    let mut context = Context::new_with(
        state.ctm,
        glyph.bbox,
        &glyph.cache,
        glyph.xref,
        glyph.settings.clone(),
        state,
        glyph.nesting_depth,
    );

    let resources = program
        .dict()
        .get(RESOURCES)
        .or_else(|| glyph.glyph.font().resources())
        .map(Resources::new);
    let resources = resources.as_ref().unwrap_or(&glyph.parent_resources);

    if is_shape_glyph {
        let mut device = Type3ShapeGlyphDevice::new(device, paint.clone());
        interpret(iter, resources, &mut context, &mut device);
    } else {
        interpret(iter, resources, &mut context, device);
    }

    Some(())
}

struct Type3ShapeGlyphDevice<'a, 'b, T: Device<'a>> {
    inner: &'b mut T,
    paint: Paint<'a>,
}

impl<'a, 'b, T: Device<'a>> Type3ShapeGlyphDevice<'a, 'b, T> {
    pub(crate) fn new(device: &'b mut T, paint: Paint<'a>) -> Self {
        Self {
            inner: device,
            paint,
        }
    }
}

// Only filling, stroking of paths and stencil masks are allowed.
impl<'a, T: Device<'a>> Device<'a> for Type3ShapeGlyphDevice<'a, '_, T> {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        let props = DrawProps {
            paint: self.paint.clone(),
            ..props
        };
        self.inner.draw_path(path, props, draw_mode);
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.inner.push_clip_path(clip_path);
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}

    fn draw_glyph_run(
        &mut self,
        glyph_run: &GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        self.inner.draw_glyph_run(glyph_run, props, draw_mode);
    }

    fn pop_clip(&mut self) {
        self.inner.pop_clip();
    }

    fn pop_transparency_group(&mut self) {}

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        if let Image::Stencil(mut s) = image {
            s.paint = self.paint.clone();
            self.inner.draw_image(Image::Stencil(s), props);
        }
    }
}
