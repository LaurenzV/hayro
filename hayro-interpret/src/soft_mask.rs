use crate::cache::Cache;
use crate::context::Context;
use crate::device::Device;
use crate::x_object::{XObject, draw_xobject};
use hayro_syntax::document::page::Resources;
use hayro_syntax::object::ObjectIdentifier;
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::name::Name;
use hayro_syntax::object::stream::Stream;
use hayro_syntax::xref::XRef;
use kurbo::Affine;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

pub enum MaskType {
    Luminosity,
    Alpha,
}

struct SoftMaskRepr<'a> {
    obj_id: ObjectIdentifier,
    group: XObject<'a>,
    mask_type: MaskType,
    parent_resources: Resources<'a>,
    root_transform: Affine,
    bbox: kurbo::Rect,
    object_cache: Cache,
    xref: &'a XRef,
}

#[derive(Clone)]
pub struct SoftMask<'a>(Arc<SoftMaskRepr<'a>>);

impl Debug for SoftMask<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SoftMask({:?})", self.0.obj_id)
    }
}

impl Hash for SoftMask<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Soft masks are unique identified by their object
        self.0.obj_id.hash(state);
    }
}

impl PartialEq for SoftMask<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.obj_id == other.0.obj_id
    }
}

impl Eq for SoftMask<'_> {}

impl<'a> SoftMask<'a> {
    pub(crate) fn new(
        dict: &Dict<'a>,
        context: &Context<'a>,
        parent_resources: Resources<'a>,
        obj_id: ObjectIdentifier,
    ) -> Option<SoftMask<'a>> {
        let group_stream = dict.get::<Stream>(G)?;
        let group = XObject::new(&group_stream)?;
        let mask_type = match dict.get::<Name>(S)?.deref() {
            LUMINOSITY => MaskType::Luminosity,
            ALPHA => MaskType::Alpha,
            _ => return None,
        };

        let context = Context::new(
            context.root_transform(),
            context.bbox(),
            context.object_cache.clone(),
            context.xref,
        );

        Some(Self(Arc::new(SoftMaskRepr {
            obj_id,
            group,
            mask_type,
            root_transform: context.root_transform(),
            bbox: context.bbox(),
            object_cache: context.object_cache.clone(),
            xref: context.xref,
            parent_resources,
        })))
    }

    pub fn interpret(&self, device: &mut impl Device) {
        let mut ctx = Context::new(
            self.0.root_transform,
            self.0.bbox,
            self.0.object_cache.clone(),
            self.0.xref,
        );
        draw_xobject(&self.0.group, &self.0.parent_resources, &mut ctx, device);
    }
}
