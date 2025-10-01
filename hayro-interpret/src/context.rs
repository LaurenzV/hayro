use crate::cache::Cache;
use crate::color::ColorSpace;
use crate::convert::convert_transform;
use crate::font::Font;
use crate::interpret::state::State;
use crate::ocg::OcgState;
use crate::{FillRule, InterpreterSettings, StrokeProps};
use hayro_syntax::content::ops::Transform;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::ObjRef;
use hayro_syntax::object::Object;
use hayro_syntax::page::Resources;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, Point};
use log::warn;
use std::collections::HashMap;

/// A context for interpreting PDF files.
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
    pub(crate) ocg_state: OcgState,
}

impl<'a> Context<'a> {
    /// Create a new context.
    pub fn new(
        initial_transform: Affine,
        bbox: kurbo::Rect,
        xref: &'a XRef,
        settings: InterpreterSettings,
    ) -> Self {
        let cache = Cache::new();
        let state = State::new(initial_transform);

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
        let ocg_state = {
            let root_ref = xref.root_id();
            let catalog = xref.get::<Dict>(root_ref).unwrap();
            OcgState::from_catalog(&catalog)
        };

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
            ocg_state,
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

    pub(crate) fn push_bbox(&mut self, bbox: kurbo::Rect) {
        let new = self.bbox().intersect(bbox);
        self.bbox.push(new);
    }

    pub(crate) fn pop_bbox(&mut self) {
        self.bbox.pop();
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
                        resources
                            .resolve_ref::<Dict>(ref_)
                            .and_then(|o| Font::new(&o, &self.settings.font_resolver))
                    })
                    .clone()
            }),
            Box::new(|c| Font::new(&c, &self.settings.font_resolver)),
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
                        .map(|o| ColorSpace::new(o, &self.object_cache))
                })
            }),
            Box::new(|c| Some(ColorSpace::new(c, &self.object_cache))),
        )?
    }

    pub(crate) fn stroke_props(&self) -> StrokeProps {
        self.get().graphics_state.stroke_props.clone()
    }

    pub(crate) fn num_states(&self) -> usize {
        self.states.len()
    }
}
