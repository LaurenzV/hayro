use super::xobject_oc;
use crate::cache::{Cache, CacheKey};
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
use std::sync::Arc;

/// Distinguishes a form's decoded content in the object cache from anything else cached for the same stream.
const FORM_CONTENT: u128 = 0x666f_726d_5f63_6f6e_7465_6e74;

/// A form's decoded content stream.
pub(crate) enum FormContent<'a> {
    Decoded(Cow<'a, [u8]>),
    /// Kept in the object cache, so that a form drawn many times, like a logo
    /// repeated on every page or a mark stamped hundreds of times on one, is
    /// only decompressed once.
    Cached(Arc<[u8]>),
}

impl AsRef<[u8]> for FormContent<'_> {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Decoded(data) => data.as_ref(),
            Self::Cached(data) => data,
        }
    }
}

pub(crate) struct FormXObject<'a> {
    pub(crate) decoded: FormContent<'a>,
    pub(crate) matrix: Affine,
    pub(crate) bbox: [f32; 4],
    is_transparency_group: bool,
    isolated: bool,
    knockout: bool,
    pub(crate) dict: Dict<'a>,
    pub(crate) resources: Option<Resources<'a>>,
}

impl<'a> FormXObject<'a> {
    pub(crate) fn new(stream: &Stream<'a>) -> Option<Self> {
        Self::with_content(stream, FormContent::Decoded(stream.decoded_data().ok()?))
    }

    /// Like `new`, but takes the decoded content from `cache` when the same
    /// form has been drawn before.
    pub(crate) fn new_cached(stream: &Stream<'a>, cache: &Cache) -> Option<Self> {
        let decoded =
            cache.get_or_insert_with((stream.cache_key(), FORM_CONTENT).cache_key(), || {
                stream
                    .decoded_data()
                    .ok()
                    .map(|data| Arc::<[u8]>::from(data.as_ref()))
            })?;

        Self::with_content(stream, FormContent::Cached(decoded))
    }

    fn with_content(stream: &Stream<'a>, decoded: FormContent<'a>) -> Option<Self> {
        let dict = stream.dict();

        let resources = dict.get(RESOURCES).map(Resources::new);

        let matrix = Affine::new(
            dict.get::<[f64; 6]>(MATRIX)
                .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        );
        let bbox = dict.get::<[f32; 4]>(BBOX)?;
        let group = dict.get::<Dict<'_>>(GROUP);
        let flag = |key| {
            group
                .as_ref()
                .and_then(|g| g.get::<bool>(key))
                .unwrap_or(false)
        };

        Some(Self {
            decoded,
            matrix,
            is_transparency_group: group.is_some(),
            isolated: flag(I),
            knockout: flag(K),
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
                context.fill_paint().alpha,
                std::mem::take(&mut context.compositing_mut().soft_mask),
                std::mem::take(&mut context.compositing_mut().blend_mode),
                self.isolated,
                self.knockout,
            );

            context.fill_paint_mut().alpha = 1.0;
            context.stroke_paint_mut().alpha = 1.0;
        }

        device.push_clip_path(&ClipPath {
            path: context.ctm()
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
            self.resources.as_ref().unwrap_or(resources),
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
