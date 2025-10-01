use std::collections::HashSet;
use crate::object::{Array, Dict, Name, ObjectIdentifier};
use crate::object::dict::keys::{BASE_STATE, D, OCGS, OCPROPERTIES, OFF, ON};

/// State tracker for Optional Content Groups (layers).
#[doc(hidden)]
pub struct OcgState {
    active_ocgs: HashSet<ObjectIdentifier>,
    // Stack of visibility states. If any element is false, content is not visible.
    // This allows O(1) visibility checks instead of iterating.
    visibility_stack: Vec<bool>,
}

impl OcgState {
    pub(crate) fn from_catalog(catalog: Option<&Dict>) -> Self {
        let active_ocgs = catalog
            .map(Self::read_default_active_ocgs)
            .unwrap_or_default();

        Self {
            active_ocgs,
            visibility_stack: Vec::new(),
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

        // Helper to read OCG refs from an array
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
                        explicitly_set.insert(id);
                    }
                }
            }
        };

        // Apply ON array - these OCGs are explicitly visible
        read_ocg_array(ON, true);

        // Apply OFF array - these OCGs are explicitly hidden
        read_ocg_array(OFF, false);

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
        let is_active = self.active_ocgs.contains(&ocg_id);
        // If already invisible, stay invisible. Otherwise use the OCG's state.
        let visible = self.is_visible() && is_active;
        self.visibility_stack.push(visible);
    }

    pub fn begin_marked_content(&mut self) {
        // Non-OCG marked content inherits parent visibility
        let visible = self.is_visible();
        self.visibility_stack.push(visible);
    }

    pub fn end_marked_content(&mut self) {
        self.visibility_stack.pop();
    }

    pub fn is_visible(&self) -> bool {
        // If stack is empty, everything is visible
        // Otherwise, check the top of the stack (most recent visibility state)
        self.visibility_stack.last().copied().unwrap_or(true)
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