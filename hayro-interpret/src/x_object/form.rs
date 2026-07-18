use super::xobject_oc;
use crate::context::Context;
use crate::device::Device;
use crate::{ClipPath, FillRule, interpret};
use hayro_syntax::content::TypedIter;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::page::Resources;
use kurbo::{Affine, Rect, Shape};
use std::borrow::Cow;

pub(crate) struct FormXObject<'a> {
    pub(crate) decoded: Cow<'a, [u8]>,
    pub(crate) matrix: Affine,
    pub(crate) bbox: [f32; 4],
    is_transparency_group: bool,
    pub(crate) dict: Dict<'a>,
    resources: Dict<'a>,
}

impl<'a> FormXObject<'a> {
    pub(crate) fn new(stream: &Stream<'a>) -> Option<Self> {
        let dict = stream.dict();

        let decoded = stream.decoded().ok()?;
        let resources = dict.get::<Dict<'_>>(RESOURCES).unwrap_or_default();

        let matrix = Affine::new(
            dict.get::<[f64; 6]>(MATRIX)
                .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        );
        let bbox = dict.get::<[f32; 4]>(BBOX)?;
        let is_transparency_group = dict.get::<Dict<'_>>(GROUP).is_some();

        Some(Self {
            decoded,
            matrix,
            is_transparency_group,
            bbox,
            dict: dict.clone(),
            resources,
        })
    }

    pub(crate) fn draw(
        &self,
        resources: &Resources<'a>,
        context: &mut Context<'a>,
        device: &mut impl Device<'a>,
    ) {
        if !context.ocg_state.is_visible() {
            return;
        }

        if !context.begin_nested_interpretation() {
            return;
        }

        let has_oc = xobject_oc(&self.dict, context);
        if !context.ocg_state.is_visible() {
            if has_oc {
                context.ocg_state.end_marked_content();
            }
            context.end_nested_interpretation();
            return;
        }

        let iter = TypedIter::new(self.decoded.as_ref());

        context.path_mut().truncate(0);
        context.save_state();
        context.pre_concat_affine(self.matrix);
        context.push_root_transform();

        if self.is_transparency_group {
            device.push_transparency_group(
                context.get().graphics_state.non_stroke_alpha,
                std::mem::take(&mut context.get_mut().graphics_state.soft_mask),
                std::mem::take(&mut context.get_mut().graphics_state.blend_mode),
            );

            context.get_mut().graphics_state.non_stroke_alpha = 1.0;
            context.get_mut().graphics_state.stroke_alpha = 1.0;
        }

        device.push_clip_path(&ClipPath {
            path: context.get().ctm
                * Rect::new(
                    self.bbox[0] as f64,
                    self.bbox[1] as f64,
                    self.bbox[2] as f64,
                    self.bbox[3] as f64,
                )
                .to_path(0.1),
            fill: FillRule::NonZero,
        });

        interpret(
            iter,
            &Resources::from_parent(self.resources.clone(), resources.clone()),
            context,
            device,
        );

        device.pop_clip();

        if self.is_transparency_group {
            device.pop_transparency_group();
        }

        context.pop_root_transform();
        context.restore_state(device);

        if has_oc {
            context.ocg_state.end_marked_content();
        }

        context.end_nested_interpretation();
    }
}
