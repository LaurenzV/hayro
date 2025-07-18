use crate::cache::Cache;
use crate::color::ColorSpace;
use crate::convert::convert_transform;
use crate::font::Font;
use crate::interpret::state::{State, TextState};
use crate::{FillProps, FillRule, InterpreterSettings, StrokeProps};
use hayro_syntax::content::ops::Transform;
use hayro_syntax::document::page::Resources;
use hayro_syntax::object::Object;
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::name::Name;
use hayro_syntax::object::r#ref::ObjRef;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, Cap, Join, Point};
use log::warn;
use smallvec::smallvec;
use std::collections::HashMap;

pub struct Context<'a> {
    states: Vec<State<'a>>,
    path: BezPath,
    sub_path_start: Point,
    last_point: Point,
    clip: Option<FillRule>,
    font_cache: HashMap<ObjRef, Option<Font<'a>>>,
    root_transforms: Vec<Affine>,
    bbox: Vec<kurbo::Rect>,
    pub(crate) settings: InterpreterSettings,
    pub(crate) object_cache: Cache,
    pub(crate) xref: &'a XRef,
}

impl<'a> Context<'a> {
    pub fn new(
        initial_transform: Affine,
        bbox: kurbo::Rect,
        cache: Cache,
        xref: &'a XRef,
        settings: InterpreterSettings,
    ) -> Self {
        let line_width = 1.0;
        let line_cap = Cap::Butt;
        let line_join = Join::Miter;
        let miter_limit = 10.0;

        let state = State {
            line_width,
            line_cap,
            line_join,
            miter_limit,
            dash_array: smallvec![],
            dash_offset: 0.0,
            ctm: initial_transform,
            non_stroke_alpha: 1.0,
            stroke_cs: ColorSpace::device_gray(),
            stroke_color: smallvec![0.0,],
            none_stroke_cs: ColorSpace::device_gray(),
            non_stroke_color: smallvec![0.0],
            stroke_alpha: 1.0,
            fill_rule: FillRule::NonZero,
            n_clips: 0,
            soft_mask: None,
            text_state: TextState::default(),
            stroke_pattern: None,
            non_stroke_pattern: None,
        };

        Self::new_with(initial_transform, bbox, cache, xref, settings, state)
    }

    pub(crate) fn new_with(
        initial_transform: Affine,
        bbox: kurbo::Rect,
        cache: Cache,
        xref: &'a XRef,
        settings: InterpreterSettings,
        state: State<'a>,
    ) -> Self {
        Self {
            states: vec![state],
            settings,
            xref,
            root_transforms: vec![initial_transform],
            last_point: Point::default(),
            sub_path_start: Point::default(),
            clip: None,
            bbox: vec![bbox],
            path: BezPath::new(),
            font_cache: HashMap::new(),
            object_cache: cache,
        }
    }

    pub(crate) fn save_state(&mut self) {
        let Some(cur) = self.states.last().cloned() else {
            warn!("attempted to save state without existing state");
            return;
        };

        self.states.push(cur);
    }

    pub(crate) fn bbox(&self) -> kurbo::Rect {
        self.bbox.last().copied().unwrap_or_else(|| {
            warn!("failed to get a bbox");

            kurbo::Rect::new(0.0, 0.0, 1.0, 1.0)
        })
    }

    pub(crate) fn push_root_transform(&mut self) {
        self.root_transforms.push(self.get().ctm);
    }

    pub(crate) fn pop_root_transform(&mut self) {
        self.root_transforms.pop();
    }

    pub(crate) fn root_transform(&self) -> Affine {
        self.root_transforms
            .last()
            .copied()
            .unwrap_or(Affine::IDENTITY)
    }

    pub(crate) fn restore_state(&mut self) {
        if self.states.len() > 1 {
            self.states.pop();
        } else {
            warn!("overflow in `restore_state");
        }
    }

    pub(crate) fn path(&self) -> &BezPath {
        &self.path
    }

    pub(crate) fn path_mut(&mut self) -> &mut BezPath {
        &mut self.path
    }

    pub(crate) fn sub_path_start(&self) -> &Point {
        &self.sub_path_start
    }

    pub(crate) fn sub_path_start_mut(&mut self) -> &mut Point {
        &mut self.sub_path_start
    }

    pub(crate) fn last_point(&self) -> &Point {
        &self.last_point
    }

    pub(crate) fn last_point_mut(&mut self) -> &mut Point {
        &mut self.last_point
    }

    pub(crate) fn clip(&self) -> &Option<FillRule> {
        &self.clip
    }

    pub(crate) fn clip_mut(&mut self) -> &mut Option<FillRule> {
        &mut self.clip
    }

    pub(crate) fn get(&self) -> &State<'a> {
        self.states.last().unwrap()
    }

    pub(crate) fn get_mut(&mut self) -> &mut State<'a> {
        self.states.last_mut().unwrap()
    }

    pub(crate) fn pre_concat_transform(&mut self, transform: Transform) {
        self.pre_concat_affine(convert_transform(transform))
    }

    pub(crate) fn pre_concat_affine(&mut self, transform: Affine) {
        self.get_mut().ctm *= transform;
    }

    pub(crate) fn get_font(&mut self, resources: &Resources<'a>, name: Name) -> Option<Font<'a>> {
        resources.get_font(
            name,
            Box::new(|ref_| {
                self.font_cache
                    .entry(ref_)
                    .or_insert_with(|| {
                        resources.resolve_ref::<Dict>(ref_).and_then(|o| {
                            Font::new(
                                &o,
                                &self.settings.font_resolver,
                                &self.settings.warning_sink,
                            )
                        })
                    })
                    .clone()
            }),
            Box::new(|c| {
                Font::new(
                    &c,
                    &self.settings.font_resolver,
                    &self.settings.warning_sink,
                )
            }),
        )
    }

    pub(crate) fn get_color_space(
        &mut self,
        resources: &Resources,
        name: Name,
    ) -> Option<ColorSpace> {
        resources.get_color_space(
            name,
            Box::new(|ref_| {
                self.object_cache.get_or_insert_with(ref_.into(), || {
                    resources
                        .resolve_ref::<Object>(ref_)
                        .map(|o| ColorSpace::new(o))
                })
            }),
            Box::new(|c| Some(ColorSpace::new(c))),
        )?
    }

    pub(crate) fn stroke_props(&self) -> StrokeProps {
        let state = self.get();

        StrokeProps {
            line_width: state.line_width,
            line_cap: state.line_cap,
            line_join: state.line_join,
            miter_limit: state.miter_limit,
            dash_array: state.dash_array.clone(),
            dash_offset: state.dash_offset,
        }
    }

    pub(crate) fn num_states(&self) -> usize {
        self.states.len()
    }

    pub(crate) fn fill_props(&self) -> FillProps {
        let state = self.get();

        FillProps {
            fill_rule: state.fill_rule,
        }
    }

    pub fn xref(&self) -> &'a XRef {
        self.xref
    }
}
