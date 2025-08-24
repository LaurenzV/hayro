use crate::cache::Cache;
use crate::context::Context;
use crate::device::Device;
use crate::interpret::state::State;
use crate::util::hash128;
use crate::x_object::{XObject, draw_xobject};
use crate::{CacheKey, InterpreterSettings};
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::ObjectIdentifier;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::page::Resources;
use hayro_syntax::xref::XRef;
use kurbo::Affine;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

/// Type type of mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskType {
    /// A luminosity mask.
    Luminosity,
    /// An alpha mask.
    Alpha,
}

struct Repr<'a> {
    obj_id: ObjectIdentifier,
    group: XObject<'a>,
    mask_type: MaskType,
    parent_resources: Resources<'a>,
    root_transform: Affine,
    bbox: kurbo::Rect,
    object_cache: Cache,
    settings: InterpreterSettings,
    xref: &'a XRef,
}

/// A soft mask.
#[derive(Clone)]
pub struct SoftMask<'a>(Arc<Repr<'a>>);

impl Debug for SoftMask<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SoftMask({:?})", self.0.obj_id)
    }
}

impl Hash for SoftMask<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Soft masks are uniquely identified by their object
        self.0.obj_id.hash(state);
    }
}

impl PartialEq for SoftMask<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.obj_id == other.0.obj_id
    }
}

impl Eq for SoftMask<'_> {}

impl CacheKey for SoftMask<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl<'a> SoftMask<'a> {
    pub(crate) fn new(
        dict: &Dict<'a>,
        context: &Context<'a>,
        parent_resources: Resources<'a>,
    ) -> Option<SoftMask<'a>> {
        // TODO: With this setup, if there is a luminosity mask and alpha mask pointing to the
        // same xobject, the ID will be the same.
        let obj_id = dict.get_ref(G)?.into();
        let group_stream = dict.get::<Stream>(G)?;
        let group = XObject::new(
            &group_stream,
            &context.settings.warning_sink,
            &context.object_cache,
        )?;
        let mask_type = match dict.get::<Name>(S)?.deref() {
            LUMINOSITY => MaskType::Luminosity,
            ALPHA => MaskType::Alpha,
            _ => return None,
        };

        Some(Self(Arc::new(Repr {
            obj_id,
            group,
            mask_type,
            root_transform: context.get().ctm,
            bbox: context.bbox(),
            object_cache: context.object_cache.clone(),
            settings: context.settings.clone(),
            xref: context.xref,
            parent_resources,
        })))
    }

    /// Interpret the contents of the mask into the given device.
    pub fn interpret(&self, device: &mut impl Device<'a>) {
        let state = State::new(self.0.root_transform);
        let mut ctx = Context::new_with(
            self.0.root_transform,
            self.0.bbox,
            self.0.object_cache.clone(),
            self.0.xref,
            self.0.settings.clone(),
            state,
        );
        draw_xobject(&self.0.group, &self.0.parent_resources, &mut ctx, device);
    }

    /// Return the object identifier of the mask.
    ///
    /// This can be used as a unique identifier for caching purposes.
    pub fn id(&self) -> ObjectIdentifier {
        self.0.obj_id
    }

    /// Return the underlying mask type.
    pub fn mask_type(&self) -> MaskType {
        self.0.mask_type
    }
}
