use hayro_syntax::object::array::ArrayIter;
use hayro_syntax::object::dict::keys::{
    BASE_STATE, D, OC, OCGS, OCMD, OCPROPERTIES, OFF, ON, P, TYPE, VE,
};
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier};
use hayro_syntax::xref::XRef;
use rustc_hash::FxHashSet;

const MAX_EXPRESSION_DEPTH: usize = 32;
const MAX_EXPRESSION_OPERANDS: usize = 4096;

pub(crate) struct OcgState {
    enabled: bool,
    inactive_ocgs: FxHashSet<ObjectIdentifier>,
    resolved_ocg_dicts: FxHashSet<ObjectIdentifier>,
    visibility_stack: Vec<bool>,
}

impl OcgState {
    pub(crate) fn from_catalog(catalog: &Dict<'_>) -> Self {
        let Some(oc_properties) = catalog.get::<Dict<'_>>(OCPROPERTIES) else {
            return Self::dummy();
        };

        let Some(config) = oc_properties.get::<Dict<'_>>(D) else {
            return Self::dummy();
        };

        let mut inactive = FxHashSet::default();

        let base_state = config
            .get::<Name<'_>>(BASE_STATE)
            .and_then(|b| BaseState::from_name(b.as_ref()));

        if base_state.unwrap_or(BaseState::On) == BaseState::Off
            && let Some(ocgs) = oc_properties.get::<Array<'_>>(OCGS)
        {
            for item in ocgs.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    inactive.insert(ref_.into());
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
            enabled: true,
            inactive_ocgs: inactive,
            resolved_ocg_dicts: FxHashSet::default(),
            visibility_stack: Vec::new(),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn is_oc_active(&mut self, dict: &Dict<'_>, xref: &XRef) -> bool {
        if !self.enabled {
            return true;
        }

        match dict.get_raw::<Object<'_>>(OC) {
            Some(MaybeRef::Ref(ref_)) => {
                let id: ObjectIdentifier = ref_.into();
                let Some(Object::Dict(oc_dict)) = xref.get::<Object<'_>>(id) else {
                    return true;
                };

                match oc_dict.get::<Name<'_>>(TYPE).as_deref() {
                    Some(OCMD) => self.is_ocmd_active(&oc_dict, xref),
                    _ => {
                        self.resolved_ocg_dicts.insert(id);
                        !self.inactive_ocgs.contains(&id)
                    }
                }
            }
            Some(MaybeRef::NotRef(Object::Dict(oc_dict))) => self.is_ocmd_active(&oc_dict, xref),
            _ => true,
        }
    }

    pub(crate) fn begin_single_oc(&mut self, ocg_id: ObjectIdentifier) {
        let is_active = !self.enabled || !self.inactive_ocgs.contains(&ocg_id);
        let visible = self.is_visible() && is_active;
        self.visibility_stack.push(visible);
    }

    pub(crate) fn begin_ocmd(&mut self, ocmd: &Dict<'_>, xref: &XRef) {
        let is_active = !self.enabled || self.is_ocmd_active(ocmd, xref);
        let visible = self.is_visible() && is_active;
        self.visibility_stack.push(visible);
    }

    pub(crate) fn begin_ocg(&mut self, props: &Dict<'_>, ref_id: ObjectIdentifier, xref: &XRef) {
        if !self.enabled {
            self.begin_marked_content();
            return;
        }

        match props.get::<Name<'_>>(TYPE).as_deref() {
            Some(OCMD) => self.begin_ocmd(props, xref),
            _ => self.begin_single_oc(ref_id),
        }
    }

    pub(crate) fn begin_marked_content(&mut self) {
        let visible = self.is_visible();
        self.visibility_stack.push(visible);
    }

    pub(crate) fn end_marked_content(&mut self) {
        self.visibility_stack.pop();
    }

    pub(crate) fn depth(&self) -> usize {
        self.visibility_stack.len()
    }

    pub(crate) fn truncate(&mut self, depth: usize) {
        self.visibility_stack.truncate(depth);
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visibility_stack.last().copied().unwrap_or(true)
    }

    fn dummy() -> Self {
        Self {
            enabled: false,
            inactive_ocgs: FxHashSet::default(),
            resolved_ocg_dicts: FxHashSet::default(),
            visibility_stack: Vec::new(),
        }
    }

    fn is_ocmd_active(&mut self, ocmd: &Dict<'_>, xref: &XRef) -> bool {
        // Fall back to `OCGs` and `P` if `VE` is malformed or exceeds our limits.
        if let Some(expression) = ocmd.get::<Array<'_>>(VE)
            && let Some(active) = self.evaluate_expression(&expression, xref)
        {
            return active;
        }

        let policy = ocmd
            .get::<Name<'_>>(P)
            .and_then(|n| OcmdPolicy::from_name(n.as_ref()))
            .unwrap_or(OcmdPolicy::AnyOn);

        match ocmd.get_raw::<Object<'_>>(OCGS) {
            Some(MaybeRef::NotRef(Object::Array(arr))) => self.eval_ocg_array(&arr, policy, xref),
            Some(MaybeRef::Ref(ref_)) => {
                let id: ObjectIdentifier = ref_.into();
                if self.resolved_ocg_dicts.contains(&id) {
                    return self.eval_single_ocg_policy(id, policy);
                }

                match xref.get::<Object<'_>>(id) {
                    Some(Object::Array(arr)) => self.eval_ocg_array(&arr, policy, xref),
                    Some(Object::Dict(_)) => {
                        self.resolved_ocg_dicts.insert(id);
                        self.eval_single_ocg_policy(id, policy)
                    }
                    _ => true,
                }
            }
            _ => true,
        }
    }

    fn eval_ocg_array(&mut self, arr: &Array<'_>, policy: OcmdPolicy, xref: &XRef) -> bool {
        let mut has_ocg = false;

        for item in arr.raw_iter() {
            let Some(ref_) = item.as_obj_ref() else {
                continue;
            };

            let id: ObjectIdentifier = ref_.into();
            if !self.is_ocg_dict(id, xref) {
                continue;
            }

            has_ocg = true;
            let active = !self.inactive_ocgs.contains(&id);

            match policy {
                OcmdPolicy::AllOn if !active => return false,
                OcmdPolicy::AnyOn if active => return true,
                OcmdPolicy::AnyOff if !active => return true,
                OcmdPolicy::AllOff if active => return false,
                _ => {}
            }
        }

        if !has_ocg {
            return true;
        }

        match policy {
            OcmdPolicy::AllOn | OcmdPolicy::AllOff => true,
            OcmdPolicy::AnyOn | OcmdPolicy::AnyOff => false,
        }
    }

    fn eval_single_ocg_policy(&self, id: ObjectIdentifier, policy: OcmdPolicy) -> bool {
        let active = !self.inactive_ocgs.contains(&id);
        match policy {
            OcmdPolicy::AllOn | OcmdPolicy::AnyOn => active,
            OcmdPolicy::AnyOff | OcmdPolicy::AllOff => !active,
        }
    }

    fn is_ocg_dict(&mut self, id: ObjectIdentifier, xref: &XRef) -> bool {
        if self.resolved_ocg_dicts.contains(&id) {
            return true;
        }

        // Don't cache misses so a later `XRef::repair` can still resolve `id`.
        if matches!(xref.get::<Object<'_>>(id), Some(Object::Dict(_))) {
            self.resolved_ocg_dicts.insert(id);
            true
        } else {
            false
        }
    }

    fn evaluate_expression(&mut self, expression: &Array<'_>, xref: &XRef) -> Option<bool> {
        let mut stack = vec![ExpressionFrame::new(expression, xref)?];
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

            match operand {
                MaybeRef::Ref(ref_) => {
                    let id: ObjectIdentifier = ref_.into();
                    if self.resolved_ocg_dicts.contains(&id) {
                        let active = !self.inactive_ocgs.contains(&id);
                        stack.last_mut()?.push(active)?;
                        continue;
                    }

                    match xref.get::<Object<'_>>(id) {
                        Some(Object::Array(nested)) => {
                            if stack.len() == MAX_EXPRESSION_DEPTH {
                                return None;
                            }

                            stack.push(ExpressionFrame::new(&nested, xref)?);
                        }
                        Some(Object::Dict(_)) => {
                            self.resolved_ocg_dicts.insert(id);
                            let active = !self.inactive_ocgs.contains(&id);
                            stack.last_mut()?.push(active)?;
                        }
                        _ => {}
                    }
                }
                MaybeRef::NotRef(Object::Array(nested)) => {
                    if stack.len() == MAX_EXPRESSION_DEPTH {
                        return None;
                    }

                    stack.push(ExpressionFrame::new(&nested, xref)?);
                }
                _ => {}
            }
        }
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

impl ExpressionOperator {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"And" => Some(Self::And),
            b"Or" => Some(Self::Or),
            b"Not" => Some(Self::Not),
            _ => None,
        }
    }
}

struct ExpressionFrame<'a> {
    operator: ExpressionOperator,
    operands: ArrayIter<'a>,
    value: Option<bool>,
}

impl<'a> ExpressionFrame<'a> {
    fn new(expression: &Array<'a>, xref: &XRef) -> Option<Self> {
        let mut operands = expression.raw_iter();
        let operator = match operands.next()? {
            MaybeRef::NotRef(Object::Name(name)) => ExpressionOperator::from_name(name.as_ref())?,
            MaybeRef::Ref(ref_) => {
                let name = xref.get::<Name<'_>>(ref_.into())?;
                ExpressionOperator::from_name(name.as_ref())?
            }
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

#[cfg(test)]
mod tests {
    use crate::font::GlyphRun;
    use crate::{
        BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
        InterpreterCache, InterpreterSettings, Paint, SoftMask, interpret_page,
    };
    use hayro_syntax::Pdf;
    use hayro_syntax::object::{Dict, ObjectIdentifier};
    use kurbo::{Affine, BezPath, Rect};
    use std::fmt::Write;

    #[derive(Default)]
    struct FillIdCollector {
        ids: Vec<u8>,
    }

    impl<'a> Device<'a> for FillIdCollector {
        fn draw_path(&mut self, _: &BezPath, props: DrawProps<'a>, _: &DrawMode) {
            if let Paint::Color(color) = props.paint {
                let id = (color.to_rgba().to_rgba8()[0] as u16 * 100 / 255) as u8;
                self.ids.push(id);
            }
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(
            &mut self,
            _: f32,
            _: Option<SoftMask<'a>>,
            _: BlendMode,
            _: bool,
            _: bool,
        ) {
        }
        fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}
        fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
        fn pop_clip(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    fn build_pdf(objects: &[(usize, &str)]) -> Vec<u8> {
        let max_id = objects.iter().map(|(id, _)| *id).max().unwrap_or(0);
        let mut out = String::from("%PDF-1.7\n");
        let mut offsets = vec![None; max_id + 1];

        for &(id, body) in objects {
            offsets[id] = Some(out.len());
            writeln!(out, "{id} 0 obj\n{body}\nendobj").unwrap();
        }

        let xref_offset = out.len();
        writeln!(out, "xref\n0 {}", max_id + 1).unwrap();
        writeln!(out, "0000000000 65535 f ").unwrap();
        for offset in &offsets[1..] {
            match offset {
                Some(offset) => writeln!(out, "{offset:010} 00000 n ").unwrap(),
                None => writeln!(out, "0000000000 65535 f ").unwrap(),
            }
        }
        writeln!(
            out,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF",
            max_id + 1
        )
        .unwrap();

        out.into_bytes()
    }

    fn drawn_ids(pdf_bytes: Vec<u8>) -> Vec<u8> {
        let pdf = Pdf::new(pdf_bytes).unwrap();
        let cache = InterpreterCache::new();
        let mut context = Context::new(
            Affine::IDENTITY,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            &cache,
            pdf.xref(),
            InterpreterSettings::default(),
        );
        let mut collector = FillIdCollector::default();
        interpret_page(&pdf.pages()[0], &mut context, &mut collector);
        collector.ids
    }

    fn draw_cmd(id: u8) -> String {
        format!("{:.2} 0 0 rg 0 0 10 10 re f", f32::from(id) / 100.0)
    }

    fn stream_obj(content: &str) -> String {
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
    }

    fn form_obj(content: &str, extra: &str) -> String {
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10]{extra} /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
    }

    #[test]
    fn ocmd_ocgs_ignores_deleted_null_and_non_dict_refs() {
        let content = [
            format!("/OC /A BDC {} EMC", draw_cmd(10)),
            format!("/OC /B BDC {} EMC", draw_cmd(20)),
            format!("/OC /C BDC {} EMC", draw_cmd(30)),
            format!("/OC /D BDC {} EMC", draw_cmd(40)),
            format!("/OC /E BDC {} EMC", draw_cmd(50)),
            format!("/OC /F BDC {} EMC", draw_cmd(60)),
            format!("/OC /G BDC {} EMC", draw_cmd(70)),
            format!("/OC /H BDC {} EMC", draw_cmd(80)),
            format!("/OC /I BDC {} EMC", draw_cmd(90)),
        ]
        .join("\n");
        let content_obj = stream_obj(&content);
        let dummy_stream = stream_obj("");

        let pdf = build_pdf(&[
            (
                1,
                "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R] /D << /OFF [6 0 R 98 0 R] >> >> >>",
            ),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Properties << /A 10 0 R /B 11 0 R /C 12 0 R /D 13 0 R /E 14 0 R /F 15 0 R /G 16 0 R /H 17 0 R /I 18 0 R >> >> >>",
            ),
            (4, &content_obj),
            (5, "<< /Type /OCG /Name (On) >>"),
            (6, "<< /Type /OCG /Name (Off) >>"),
            (7, "null"),
            (8, &dummy_stream),
            (9, "[6 0 R 99 0 R]"),
            (10, "<< /Type /OCMD /OCGs [6 0 R 99 0 R] /P /AnyOn >>"),
            (11, "<< /Type /OCMD /OCGs [6 0 R 7 0 R] /P /AnyOn >>"),
            (12, "<< /Type /OCMD /OCGs [6 0 R 8 0 R] /P /AnyOn >>"),
            (13, "<< /Type /OCMD /OCGs [99 0 R] /P /AllOff >>"),
            (14, "<< /Type /OCMD /OCGs 99 0 R /P /AllOff >>"),
            (15, "<< /Type /OCMD /OCGs [6 0 R 99 0 R] /P /AllOff >>"),
            (16, "<< /Type /OCMD /OCGs [5 0 R 98 0 R] /P /AnyOff >>"),
            (17, "<< /Type /OCMD /OCGs 9 0 R /P /AnyOn >>"),
            (18, "<< /Type /OCMD /VE [/Or 6 0 R 99 0 R] >>"),
        ]);

        assert_eq!(drawn_ids(pdf), vec![40, 50, 60]);
    }

    #[test]
    fn visibility_stack_is_isolated_across_streams_and_annotations() {
        let page_content = format!("/OC /Off BDC {}", draw_cmd(10));
        let page_stream = stream_obj(&page_content);
        let annot1_stream = form_obj(&format!("/OC /Off BDC {}", draw_cmd(20)), "");
        let annot2_stream = form_obj(&draw_cmd(30), "");

        let pdf = build_pdf(&[
            (
                1,
                "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R] /D << /OFF [6 0 R] >> >> >>",
            ),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Properties << /On 5 0 R /Off 6 0 R >> >> /Annots [7 0 R 8 0 R 9 0 R] >>",
            ),
            (4, &page_stream),
            (5, "<< /Type /OCG /Name (On) >>"),
            (6, "<< /Type /OCG /Name (Off) >>"),
            (
                7,
                "<< /Type /Annot /Subtype /Square /Rect [0 0 10 10] /AP << /N 10 0 R >> >>",
            ),
            (
                8,
                "<< /Type /Annot /Subtype /Square /Rect [10 0 20 10] /OC 6 0 R /AP << /N 11 0 R >> >>",
            ),
            (
                9,
                "<< /Type /Annot /Subtype /Square /Rect [20 0 30 10] /AP << /N 11 0 R >> >>",
            ),
            (10, &annot1_stream),
            (11, &annot2_stream),
        ]);

        assert_eq!(drawn_ids(pdf), vec![30]);
    }

    #[test]
    fn form_unmatched_emc_and_unclosed_bdc_cannot_corrupt_caller_visibility() {
        let page_content = format!("/OC /On BDC {} /Fm1 Do {} EMC", draw_cmd(10), draw_cmd(30));
        let page_stream = stream_obj(&page_content);
        let form_stream = form_obj(
            &format!("EMC /OC /Off BDC {}", draw_cmd(20)),
            " /Resources << /Properties << /Off 6 0 R >> >>",
        );

        let pdf = build_pdf(&[
            (
                1,
                "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R] /D << /OFF [6 0 R] >> >> >>",
            ),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Properties << /On 5 0 R /Off 6 0 R >> /XObject << /Fm1 7 0 R >> >> >>",
            ),
            (4, &page_stream),
            (5, "<< /Type /OCG /Name (On) >>"),
            (6, "<< /Type /OCG /Name (Off) >>"),
            (7, &form_stream),
        ]);

        assert_eq!(drawn_ids(pdf), vec![10, 30]);
    }

    #[test]
    fn ve_resolves_indirect_operator_names() {
        let content = [
            format!("/OC /A BDC {} EMC", draw_cmd(10)),
            format!("/OC /B BDC {} EMC", draw_cmd(20)),
            format!("/OC /C BDC {} EMC", draw_cmd(30)),
            format!("/OC /D BDC {} EMC", draw_cmd(40)),
        ]
        .join("\n");
        let page_stream = stream_obj(&content);

        let pdf = build_pdf(&[
            (
                1,
                "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R] /D << /OFF [6 0 R] >> >> >>",
            ),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Properties << /A 10 0 R /B 11 0 R /C 12 0 R /D 13 0 R >> >> >>",
            ),
            (4, &page_stream),
            (5, "<< /Type /OCG /Name (On) >>"),
            (6, "<< /Type /OCG /Name (Off) >>"),
            (7, "/Not"),
            (8, "/And"),
            (9, "123"),
            (10, "<< /Type /OCMD /VE [7 0 R 5 0 R] >>"),
            (11, "<< /Type /OCMD /VE [7 0 R 6 0 R] >>"),
            (12, "<< /Type /OCMD /VE [8 0 R 5 0 R [7 0 R 6 0 R]] >>"),
            (13, "<< /Type /OCMD /VE [9 0 R 5 0 R] /OCGs 6 0 R >>"),
        ]);

        assert_eq!(drawn_ids(pdf), vec![20, 30]);
    }

    #[test]
    fn absent_oc_properties_ignores_oc_without_triggering_xref_repair() {
        let annot1_stream = form_obj(&draw_cmd(10), "");
        let annot2_stream = form_obj(&draw_cmd(20), "");
        let annot3_stream = form_obj(&draw_cmd(50), "");
        let form_stream = form_obj(&draw_cmd(30), " /OC << /Type /OCMD /VE [/Not 5 0 R] >>");
        let page_stream = stream_obj(&format!("/OC /Neg BDC {} EMC /Fm1 Do", draw_cmd(40)));
        let shadow_stream = form_obj(&draw_cmd(90), "");

        let mut out = String::from("%PDF-1.7\n");
        let mut offsets = [0_usize; 15];
        let base_objects: [(usize, &str); 12] = [
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Properties << /Neg 11 0 R >> /XObject << /Fm1 12 0 R >> >> /Annots [6 0 R 7 0 R 13 0 R] >>",
            ),
            (4, &page_stream),
            (5, "<< /Type /OCG /Name (L1) >>"),
            (
                6,
                "<< /Type /Annot /Subtype /Square /Rect [0 0 10 10] /OC 10 0 R /AP << /N 8 0 R >> >>",
            ),
            (
                7,
                "<< /Type /Annot /Subtype /Square /Rect [10 0 20 10] /AP << /N 9 0 R >> >>",
            ),
            (8, &annot1_stream),
            (11, "<< /Type /OCMD /VE [/Not 5 0 R] >>"),
            (12, &form_stream),
            (
                13,
                "<< /Type /Annot /Subtype /Square /Rect [20 0 30 10] /OC << /Type /OCMD /VE [/Not 5 0 R] >> /AP << /N 14 0 R >> >>",
            ),
            (14, &annot3_stream),
        ];

        for (id, body) in base_objects {
            offsets[id] = out.len();
            writeln!(out, "{id} 0 obj\n{body}\nendobj").unwrap();
        }
        offsets[9] = out.len();
        writeln!(out, "9 0 obj\n{annot2_stream}\nendobj").unwrap();
        // Unreferenced duplicate of object 9; only visible if `XRef::repair` scans the file.
        writeln!(out, "9 0 obj\n{shadow_stream}\nendobj").unwrap();

        let xref_offset = out.len();
        writeln!(out, "xref\n0 15").unwrap();
        writeln!(out, "0000000000 65535 f ").unwrap();
        for (id, offset) in offsets.iter().enumerate().skip(1) {
            if id == 10 {
                // Corrupt offset for 10 0 R; resolving it would trigger `XRef::repair`.
                writeln!(out, "0000000001 00000 n ").unwrap();
            } else {
                writeln!(out, "{offset:010} 00000 n ").unwrap();
            }
        }
        writeln!(
            out,
            "trailer\n<< /Size 15 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF"
        )
        .unwrap();

        let pdf = Pdf::new(out.into_bytes()).unwrap();
        let cache = InterpreterCache::new();
        let mut context = Context::new(
            Affine::IDENTITY,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            &cache,
            pdf.xref(),
            InterpreterSettings::default(),
        );
        let mut collector = FillIdCollector::default();
        interpret_page(&pdf.pages()[0], &mut context, &mut collector);
        assert_eq!(collector.ids, vec![40, 30, 10, 20, 50]);

        let _ = pdf.xref().get::<Dict<'_>>(ObjectIdentifier::new(10, 0));
        let mut repaired_collector = FillIdCollector::default();
        interpret_page(&pdf.pages()[0], &mut context, &mut repaired_collector);
        assert_eq!(repaired_collector.ids, vec![40, 30, 10, 90, 50]);
    }

    #[test]
    fn ocg_cache_does_not_cache_xref_misses_across_repair() {
        let mut out = String::from("%PDF-1.7\n");
        let mut offsets = [0_usize; 7];
        let objects: [(usize, &str); 6] = [
            (
                1,
                "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R] /D << /OFF [5 0 R] >> >> >>",
            ),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>"),
            (4, "<< /Type /OCMD /OCGs [5 0 R] /P /AnyOn >>"),
            (5, "<< /Type /OCG /Name (Off) >>"),
            (6, "<< /Dummy true >>"),
        ];

        for (id, body) in objects {
            offsets[id] = out.len();
            writeln!(out, "{id} 0 obj\n{body}\nendobj").unwrap();
        }

        let xref_offset = out.len();
        writeln!(out, "xref\n0 7").unwrap();
        writeln!(out, "0000000000 65535 f ").unwrap();
        for (id, offset) in offsets.iter().enumerate().skip(1) {
            match id {
                5 => writeln!(out, "0000000000 65535 f ").unwrap(),
                6 => writeln!(out, "0000000001 00000 n ").unwrap(),
                _ => writeln!(out, "{offset:010} 00000 n ").unwrap(),
            }
        }
        writeln!(
            out,
            "trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF"
        )
        .unwrap();

        let pdf = Pdf::new(out.into_bytes()).unwrap();
        let xref = pdf.xref();
        let catalog = xref.get::<Dict<'_>>(xref.root_id()).unwrap();
        let ocmd = xref.get::<Dict<'_>>(ObjectIdentifier::new(4, 0)).unwrap();
        let mut state = super::OcgState::from_catalog(&catalog);

        assert!(state.is_ocmd_active(&ocmd, xref));
        assert!(xref.get::<Dict<'_>>(ObjectIdentifier::new(6, 0)).is_some());
        assert!(!state.is_ocmd_active(&ocmd, xref));
    }
}
