use hayro_syntax::object::dict::keys::{BASE_STATE, D, OCGS, OCPROPERTIES, OFF, ON};
use hayro_syntax::object::{Array, Dict, Name, ObjectIdentifier};
use std::collections::HashSet;

pub(crate) struct OcgState {
    inactive_ocgs: HashSet<ObjectIdentifier>,
    visibility_stack: Vec<bool>,
}

impl OcgState {
    fn dummy() -> OcgState {
        OcgState {
            inactive_ocgs: Default::default(),
            visibility_stack: vec![],
        }
    }

    pub(crate) fn from_catalog(catalog: &Dict) -> Self {
        let Some(oc_properties) = catalog.get::<Dict>(OCPROPERTIES) else {
            return Self::dummy();
        };

        let Some(config) = oc_properties.get::<Dict>(D) else {
            return Self::dummy();
        };

        let mut inactive = HashSet::new();

        let base_state = config
            .get::<Name>(BASE_STATE)
            .and_then(|b| BaseState::from_name(b.as_ref()));

        if base_state.unwrap_or(BaseState::On) == BaseState::Off
            && let Some(ocgs) = oc_properties.get::<Array>(OCGS)
        {
            for item in ocgs.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    let id: ObjectIdentifier = ref_.into();
                    inactive.insert(id);
                }
            }
        }

        let mut read_ocg_array = |key, insert_active: bool| {
            if let Some(arr) = config.get::<Array>(key) {
                for item in arr.raw_iter() {
                    if let Some(ref_) = item.as_obj_ref() {
                        let id: ObjectIdentifier = ref_.into();
                        if insert_active {
                            inactive.remove(&id);
                        } else {
                            inactive.insert(id);
                        }
                    }
                }
            }
        };

        read_ocg_array(ON, true);
        read_ocg_array(OFF, false);

        Self {
            inactive_ocgs: inactive,
            visibility_stack: Vec::new(),
        }
    }

    pub(crate) fn begin_ocg(&mut self, ocg_id: ObjectIdentifier) {
        let is_active = !self.inactive_ocgs.contains(&ocg_id);
        let visible = self.is_visible() && is_active;
        self.visibility_stack.push(visible);
    }

    pub(crate) fn begin_marked_content(&mut self) {
        let visible = self.is_visible();
        self.visibility_stack.push(visible);
    }

    pub(crate) fn end_marked_content(&mut self) {
        self.visibility_stack.pop();
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visibility_stack.last().copied().unwrap_or(true)
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum BaseState {
    On,
    Off,
    Unchanged,
}

impl BaseState {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"ON" => Some(BaseState::On),
            b"OFF" => Some(BaseState::Off),
            b"Unchanged" => Some(BaseState::Unchanged),
            _ => None,
        }
    }
}
