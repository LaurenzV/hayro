use std::collections::HashSet;
use hayro_syntax::object::{Array, Dict, Name, ObjectIdentifier};
use hayro_syntax::object::dict::keys::{BASE_STATE, D, OCGS, OCPROPERTIES, OFF, ON};

pub(crate) struct OcgState {
    active_ocgs: HashSet<ObjectIdentifier>,
    stack: Vec<Option<ObjectIdentifier>>,
}

impl OcgState {
    pub fn new(catalog: &Dict) -> Self {
        let active_ocgs = Self::read_default_active_ocgs(catalog);

        Self {
            active_ocgs,
            stack: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self {
            active_ocgs: HashSet::new(),
            stack: Vec::new(),
        }
    }

    fn read_default_active_ocgs(catalog: &Dict) -> HashSet<ObjectIdentifier> {
        let mut active = HashSet::new();

        let Some(ocproperties) = catalog.get::<Dict>(OCPROPERTIES) else {
            return active;
        };

        // Get the D (default configuration) dictionary
        let Some(config) = ocproperties.get::<Dict>(D) else {
            return active;
        };

        let base_state = config.get::<Name>(BASE_STATE)
            .and_then(|b| BaseState::from_name(b.as_ref()))
            .unwrap_or(BaseState::On);

        // Collect which OCGs are explicitly mentioned in ON or OFF
        let mut explicitly_set = HashSet::new();

        // First, apply ON array - these OCGs are explicitly visible
        if let Some(on_arr) = config.get::<Array>(ON) {
            for item in on_arr.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    let id: ObjectIdentifier = ref_.into();
                    active.insert(id);
                    explicitly_set.insert(id);
                }
            }
        }

        // Then, apply OFF array - these OCGs are explicitly hidden
        if let Some(off_arr) = config.get::<Array>(OFF) {
            for item in off_arr.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    let id: ObjectIdentifier = ref_.into();
                    active.remove(&id);
                    explicitly_set.insert(id);
                }
            }
        }

        // For OCGs not explicitly set, apply BaseState
        if let BaseState::On = base_state {
            if let Some(ocgs) = ocproperties.get::<Array>(OCGS) {
                for item in ocgs.raw_iter() {
                    if let Some(ref_) = item.as_obj_ref() {
                        let id: ObjectIdentifier = ref_.into();
                        if !explicitly_set.contains(&id) {
                            active.insert(id);
                        }
                    }
                }
            }
        }

        active
    }

    pub fn begin_ocg(&mut self, ocg_id: ObjectIdentifier) {
        self.stack.push(Some(ocg_id));
    }

    pub fn begin_marked_content(&mut self) {
        self.stack.push(None);
    }

    pub fn end_marked_content(&mut self) {
        self.stack.pop();
    }

    pub fn is_visible(&self) -> bool {
        for item in &self.stack {
            if let Some(ocg_id) = item {
                if !self.active_ocgs.contains(ocg_id) {
                    return false;
                }
            }
        }
        true
    }
}

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