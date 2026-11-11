use hayro_syntax::object::array::ArrayIter;
use hayro_syntax::object::dict::keys::{
    BASE_STATE, D, OC, OCGS, OCMD, OCPROPERTIES, OFF, ON, P, TYPE, VE,
};
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier};
use hayro_syntax::xref::XRef;
use std::collections::HashSet;

/// The maximum number of nested levels of a visibility expression.
const MAX_EXPRESSION_DEPTH: usize = 32;
/// The maximum number of operands of a visibility expression, counted over all levels.
const MAX_EXPRESSION_OPERANDS: usize = 4096;

pub(crate) struct OcgState {
    inactive_ocgs: HashSet<ObjectIdentifier>,
    visibility_stack: Vec<bool>,
}

impl OcgState {
    fn dummy() -> Self {
        Self {
            inactive_ocgs: HashSet::default(),
            visibility_stack: vec![],
        }
    }

    pub(crate) fn from_catalog(catalog: &Dict<'_>) -> Self {
        let Some(oc_properties) = catalog.get::<Dict<'_>>(OCPROPERTIES) else {
            return Self::dummy();
        };

        let Some(config) = oc_properties.get::<Dict<'_>>(D) else {
            return Self::dummy();
        };

        let mut inactive = HashSet::new();

        let base_state = config
            .get::<Name<'_>>(BASE_STATE)
            .and_then(|b| BaseState::from_name(b.as_ref()));

        if base_state.unwrap_or(BaseState::On) == BaseState::Off
            && let Some(ocgs) = oc_properties.get::<Array<'_>>(OCGS)
        {
            for item in ocgs.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    let id: ObjectIdentifier = ref_.into();
                    inactive.insert(id);
                }
            }
        }

        let mut read_ocg_array = |key, insert_active: bool| {
            if let Some(arr) = config.get::<Array<'_>>(key) {
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

    pub(crate) fn begin_single_oc(&mut self, ocg_id: ObjectIdentifier) {
        let is_active = !self.inactive_ocgs.contains(&ocg_id);
        let visible = self.is_visible() && is_active;
        self.visibility_stack.push(visible);
    }

    pub(crate) fn begin_ocmd(&mut self, ocmd: &Dict<'_>, xref: &XRef) {
        let visible = self.is_visible() && self.is_ocmd_active(ocmd, xref);
        self.visibility_stack.push(visible);
    }

    pub(crate) fn begin_ocg(&mut self, props: &Dict<'_>, ref_id: ObjectIdentifier, xref: &XRef) {
        match props.get::<Name<'_>>(TYPE).as_deref() {
            Some(OCMD) => self.begin_ocmd(props, xref),
            _ => self.begin_single_oc(ref_id),
        }
    }

    /// Whether the optional content entry `OC` of `dict` (for example an annotation)
    /// is active. A dictionary without such an entry is always active.
    pub(crate) fn is_oc_active(&self, dict: &Dict<'_>, xref: &XRef) -> bool {
        let Some(oc_dict) = dict.get::<Dict<'_>>(OC) else {
            return true;
        };

        match oc_dict.get::<Name<'_>>(TYPE).as_deref() {
            Some(OCMD) => self.is_ocmd_active(&oc_dict, xref),
            _ => dict
                .get_ref(OC)
                .is_none_or(|ref_| !self.inactive_ocgs.contains(&ref_.into())),
        }
    }

    fn is_ocmd_active(&self, ocmd: &Dict<'_>, xref: &XRef) -> bool {
        // 8.11.2.2: A visibility expression takes precedence over `OCGs` and `P`.
        // If the expression is malformed or exceeds our limits, we fall back to `OCGs`
        // and `P`, like a processor that doesn't support visibility expressions would.
        if let Some(expression) = ocmd.get::<Array<'_>>(VE)
            && let Some(active) = self.evaluate_expression(&expression, xref)
        {
            return active;
        }

        let policy = ocmd
            .get::<Name<'_>>(P)
            .and_then(|n| OcmdPolicy::from_name(n.as_ref()))
            .unwrap_or(OcmdPolicy::AnyOn);

        let mut ocg_ids: Vec<ObjectIdentifier> = Vec::new();

        if let Some(arr) = ocmd.get::<Array<'_>>(OCGS) {
            for item in arr.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    ocg_ids.push(ref_.into());
                }
            }
        } else if let Some(ref_) = ocmd.get_ref(OCGS) {
            ocg_ids.push(ref_.into());
        }

        if ocg_ids.is_empty() {
            true
        } else {
            match policy {
                OcmdPolicy::AllOn => ocg_ids.iter().all(|id| !self.inactive_ocgs.contains(id)),
                OcmdPolicy::AnyOn => ocg_ids.iter().any(|id| !self.inactive_ocgs.contains(id)),
                OcmdPolicy::AnyOff => ocg_ids.iter().any(|id| self.inactive_ocgs.contains(id)),
                OcmdPolicy::AllOff => ocg_ids.iter().all(|id| self.inactive_ocgs.contains(id)),
            }
        }
    }

    /// Evaluate a visibility expression (8.11.2.2) with an explicit stack.
    ///
    /// Returns `None` if the expression is malformed, nested deeper than
    /// `MAX_EXPRESSION_DEPTH` levels or has more than `MAX_EXPRESSION_OPERANDS`
    /// operands.
    fn evaluate_expression(&self, expression: &Array<'_>, xref: &XRef) -> Option<bool> {
        let mut stack = vec![ExpressionFrame::new(expression)?];
        let mut operands = 0;

        loop {
            let frame = stack.last_mut()?;

            let Some(operand) = frame.operands.next() else {
                let value = frame.finish()?;
                stack.pop();

                match stack.last_mut() {
                    Some(parent) => parent.push(value)?,
                    None => return Some(value),
                }

                continue;
            };

            operands += 1;
            if operands > MAX_EXPRESSION_OPERANDS {
                return None;
            }

            let (id, operand) = match operand {
                MaybeRef::Ref(ref_) => (Some(ref_.into()), xref.get::<Object<'_>>(ref_.into())),
                MaybeRef::NotRef(object) => (None, Some(object)),
            };

            match (operand, id) {
                (Some(Object::Array(nested)), _) => {
                    if stack.len() == MAX_EXPRESSION_DEPTH {
                        return None;
                    }

                    stack.push(ExpressionFrame::new(&nested)?);
                }
                // Optional content groups must be indirect objects.
                (Some(Object::Dict(_)), Some(id)) => {
                    let active = !self.inactive_ocgs.contains(&id);
                    stack.last_mut()?.push(active)?;
                }
                // Other operands, such as references to objects that don't exist, are
                // treated as null and ignored.
                _ => {}
            }
        }
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

impl Default for OcgState {
    fn default() -> Self {
        Self::dummy()
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
            b"ON" => Some(Self::On),
            b"OFF" => Some(Self::Off),
            b"Unchanged" => Some(Self::Unchanged),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum ExpressionOperator {
    And,
    Or,
    Not,
}

/// One level of a visibility expression that is being evaluated.
struct ExpressionFrame<'a> {
    operator: ExpressionOperator,
    operands: ArrayIter<'a>,
    /// The operator applied to the values of the operands evaluated so far.
    value: Option<bool>,
}

impl<'a> ExpressionFrame<'a> {
    fn new(expression: &Array<'a>) -> Option<Self> {
        let mut operands = expression.raw_iter();
        let operator = match operands.next()? {
            MaybeRef::NotRef(Object::Name(name)) => match name.as_ref() {
                b"And" => ExpressionOperator::And,
                b"Or" => ExpressionOperator::Or,
                b"Not" => ExpressionOperator::Not,
                _ => return None,
            },
            _ => return None,
        };

        Some(Self {
            operator,
            operands,
            value: None,
        })
    }

    fn push(&mut self, operand: bool) -> Option<()> {
        self.value = Some(match (self.operator, self.value) {
            (ExpressionOperator::And, value) => value.unwrap_or(true) && operand,
            (ExpressionOperator::Or, value) => value.unwrap_or(false) || operand,
            (ExpressionOperator::Not, None) => !operand,
            // `Not` takes exactly one operand.
            (ExpressionOperator::Not, Some(_)) => return None,
        });

        Some(())
    }

    fn finish(&self) -> Option<bool> {
        match (self.operator, self.value) {
            (_, Some(value)) => Some(value),
            (ExpressionOperator::Not, None) => None,
            // Treat an empty `And` or `Or` like an OCMD without groups, which is visible.
            (_, None) => Some(true),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum OcmdPolicy {
    AllOn,
    AnyOn,
    AnyOff,
    AllOff,
}

impl OcmdPolicy {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"AllOn" => Some(Self::AllOn),
            b"AnyOn" => Some(Self::AnyOn),
            b"AnyOff" => Some(Self::AnyOff),
            b"AllOff" => Some(Self::AllOff),
            _ => None,
        }
    }
}
