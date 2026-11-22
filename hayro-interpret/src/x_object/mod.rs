mod decode;
mod form;
mod image;
pub(crate) mod soft_mask;

pub(crate) use form::FormXObject;
pub(crate) use image::ImageXObject;

use crate::InterpreterWarning;
use crate::context::Context;
use crate::device::Device;
use hayro_syntax::object::Name;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::{Array, Dict};
use hayro_syntax::page::Resources;
use std::ops::Deref;

pub(crate) enum XObject<'a> {
    FormXObject(FormXObject<'a>),
    ImageXObject(ImageXObject<'a>),
}

impl<'a> XObject<'a> {
    pub(crate) fn new(
        stream: &Stream<'a>,
        resources: &Resources<'a>,
        context: &mut Context<'a>,
    ) -> Option<Self> {
        let dict = stream.dict();
        match dict.get::<Name<'_>>(SUBTYPE)?.deref() {
            IMAGE => {
                let alternate = display_alternate(dict, context)?;
                let source = alternate.as_ref().unwrap_or(stream);
                let warning_sink = context.settings.warning_sink.clone();
                let cache = context.interpreter_cache.object_cache.clone();
                let transfer = context.compositing().transfer_function.clone();
                let mut image = ImageXObject::new(
                    source,
                    |name| context.get_color_space(resources, name),
                    &warning_sink,
                    &cache,
                    transfer,
                )?;
                // ISO 32000-2, 8.9.5.4: only the alternate entry's /OC applies.
                image.selected_as_alternate = alternate.is_some();
                Some(Self::ImageXObject(image))
            }
            FORM => Some(Self::FormXObject(FormXObject::new(stream)?)),
            _ => None,
        }
    }

    pub(crate) fn draw(
        &self,
        resources: &Resources<'a>,
        context: &mut Context<'a>,
        device: &mut impl Device<'a>,
    ) {
        match self {
            Self::FormXObject(form) => form.draw(resources, context, device),
            Self::ImageXObject(image) => image.draw(context, device),
        }
    }
}

fn display_alternate<'a>(base: &Dict<'a>, context: &mut Context<'a>) -> Option<Option<Stream<'a>>> {
    if base.contains_key(OC) {
        return Some(None);
    }
    let Some(entries) = base.get::<Array<'_>>(b"Alternates") else {
        return Some(None);
    };
    const MAX_ALTERNATES: usize = 4096;
    if entries.raw_iter().nth(MAX_ALTERNATES).is_some() {
        (context.settings.warning_sink)(InterpreterWarning::ImageAlternateLimit);
        return None;
    }
    for entry in entries.iter::<Dict<'_>>() {
        if xobject_oc(&entry, context) {
            let visible = context.ocg_state.is_visible();
            context.ocg_state.end_marked_content();
            if visible {
                return entry.get::<Stream<'_>>(IMAGE).map(Some);
            }
        }
    }
    Some(None)
}

fn xobject_oc(dict: &Dict<'_>, context: &mut Context<'_>) -> bool {
    let Some(oc_dict) = dict.get::<Dict<'_>>(OC) else {
        return false;
    };

    if let Some(oc_ref) = dict.get_ref(OC) {
        context.ocg_state.begin_ocg(&oc_dict, oc_ref.into());
    } else {
        context.ocg_state.begin_ocmd(&oc_dict);
    }

    true
}
