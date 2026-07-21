mod decode;
mod form;
mod image;
pub(crate) mod soft_mask;

pub(crate) use form::FormXObject;
pub(crate) use image::ImageXObject;

use crate::WarningSinkFn;
use crate::cache::Cache;
use crate::color::ColorSpace;
use crate::context::Context;
use crate::device::Device;
use crate::function::Function;
use crate::interpret::state::ActiveTransferFunction;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::page::Resources;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

/// A transfer function.
#[derive(Clone)]
pub struct TransferFunction {
    function: Function,
    samples: Arc<OnceLock<[u8; 256]>>,
}

impl TransferFunction {
    fn new(function: Function) -> Self {
        Self {
            function,
            samples: Arc::new(OnceLock::new()),
        }
    }

    /// Apply the transfer function to a buffer of values.
    pub fn apply_to(&self, values: &mut [u8]) {
        self.apply_to_stride(values, 1);
    }

    fn apply_to_stride(&self, values: &mut [u8], stride: usize) {
        let samples = self.samples();

        for value in values.iter_mut().step_by(stride) {
            *value = samples[*value as usize];
        }
    }

    fn samples(&self) -> &[u8; 256] {
        self.samples.get_or_init(|| {
            std::array::from_fn(|sample| {
                self.function
                    .eval(smallvec::smallvec![sample as f32 / 255.0])
                    .and_then(|output| output.first().copied())
                    .map(|output| (output * 255.0 + 0.5) as u8)
                    .unwrap_or(sample as u8)
            })
        })
    }
}

pub(crate) enum XObject<'a> {
    FormXObject(FormXObject<'a>),
    ImageXObject(ImageXObject<'a>),
}

impl<'a> XObject<'a> {
    pub(crate) fn new(
        stream: &Stream<'a>,
        resolve_cs: impl FnOnce(&Name<'_>) -> Option<ColorSpace>,
        warning_sink: &WarningSinkFn,
        cache: &Cache,
        transfer_function: Option<ActiveTransferFunction>,
    ) -> Option<Self> {
        let dict = stream.dict();
        match dict.get::<Name<'_>>(SUBTYPE)?.deref() {
            IMAGE => Some(Self::ImageXObject(ImageXObject::new(
                stream,
                resolve_cs,
                warning_sink,
                cache,
                transfer_function,
            )?)),
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
