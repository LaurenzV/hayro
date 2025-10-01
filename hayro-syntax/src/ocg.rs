use std::collections::HashSet;
use crate::object::{Array, Dict, Name, ObjectIdentifier};
use crate::object::dict::keys::{BASE_STATE, D, OCGS, OCPROPERTIES, OFF, ON};

#[doc(hidden)]
pub struct OcgState {
    active_ocgs: HashSet<ObjectIdentifier>,
    all_active: bool,
    visibility_stack: Vec<bool>,
}

impl OcgState {
    fn dummy() -> OcgState {
        OcgState {
            active_ocgs: Default::default(),
            all_active: true,
            visibility_stack: vec![],
        }
    }
    
    pub(crate) fn from_catalog(catalog: &Dict) -> Self {
        let active_ocgs = HashSet::new();
        
        let Some(ocproperties) = catalog.get::<Dict>(OCPROPERTIES) else {
            return Self::dummy();
        };

        let Some(config) = ocproperties.get::<Dict>(D) else {
            return Self::dummy();
        };
        
        let mut active = HashSet::new();

        let base_state = config.get::<Name>(BASE_STATE)
            .and_then(|b| BaseState::from_name(b.as_ref()));
        
        if base_state == Some(BaseState::On) && let Some(ocgs) = ocproperties.get::<Array>(OCGS) {
            for item in  ocgs.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    let id: ObjectIdentifier = ref_.into();
                    active.insert(id);
                }
            }
        }
        
        let mut read_ocg_array = |key, insert_active: bool| {
            if let Some(arr) = config.get::<Array>(key) {
                for item in arr.raw_iter() {
                    if let Some(ref_) = item.as_obj_ref() {
                        let id: ObjectIdentifier = ref_.into();
                        if insert_active {
                            active.insert(id);
                        } else {
                            active.remove(&id);
                        }
                    }
                }
            }
        };

        read_ocg_array(ON, true);
        read_ocg_array(OFF, false);
        
        Self {
            active_ocgs,
            all_active: false,
            visibility_stack: Vec::new(),
        }
    }

    pub fn begin_ocg(&mut self, ocg_id: ObjectIdentifier) {
        let is_active = self.active_ocgs.contains(&ocg_id) || self.all_active;
        let visible = self.is_visible() && is_active;
        self.visibility_stack.push(visible);
    }

    pub fn begin_marked_content(&mut self) {
        let visible = self.is_visible();
        self.visibility_stack.push(visible);
    }

    pub fn end_marked_content(&mut self) {
        self.visibility_stack.pop();
    }

    pub fn is_visible(&self) -> bool {
        self.visibility_stack.last().copied().unwrap_or(true)
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum BaseState {
    On,
    Off,
    Unchanged
}

impl BaseState {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"ON" => Some(BaseState::On),
            b"OFF" => Some(BaseState::Off),
            b"Unchanged" => Some(BaseState::Unchanged),
            _ => None
        }
    }
}