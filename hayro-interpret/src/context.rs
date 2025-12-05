use crate::cache::Cache;
use crate::color::ColorSpace;
use crate::convert::convert_transform;
use crate::font::Font;
use crate::interpret::state::{ClipType, State};
use crate::ocg::OcgState;
use crate::util::Float64Ext;
use crate::{ClipPath, Device, FillRule, InterpreterSettings, StrokeProps};
use hayro_syntax::content::ops::Transform;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::ObjRef;
use hayro_syntax::object::Object;
use hayro_syntax::page::Resources;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};
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
    bbox: Vec<Rect>,
    pub(crate) settings: InterpreterSettings,
    pub(crate) object_cache: Cache,
    pub(crate) xref: &'a XRef,
    pub(crate) ocg_state: OcgState,
}

impl<'a> Context<'a> {
    /// Create a new context.
    pub fn new(
        initial_transform: Affine,
        bbox: Rect,
        xref: &'a XRef,
        settings: InterpreterSettings,
    ) -> Self {
        let cache = Cache::new();
        let state = State::new(initial_transform);

        Self::new_with(initial_transform, bbox, cache, xref, settings, state)
    }

    pub(crate) fn new_with(
        initial_transform: Affine,
        bbox: Rect,
        cache: Cache,
        xref: &'a XRef,
        settings: InterpreterSettings,
        state: State<'a>,
    ) -> Self {
        let ocg_state = {
            let root_ref = xref.root_id();
            xref.get::<Dict<'_>>(root_ref)
                .map(|catalog| OcgState::from_catalog(&catalog))
                .unwrap_or_default()
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

    pub(crate) fn bbox(&self) -> Rect {
        self.bbox.last().copied().unwrap_or_else(|| {
            warn!("failed to get a bbox");

            Rect::new(0.0, 0.0, 1.0, 1.0)
        })
    }

    fn push_bbox(&mut self, bbox: Rect) {
        let new = self.bbox().intersect(bbox);
        self.bbox.push(new);
    }

    pub(crate) fn push_clip_path(
        &mut self,
        clip_path: BezPath,
        fill: FillRule,
        device: &mut impl Device<'a>,
    ) {
        if let Some(clip_rect) = path_as_rect(&clip_path) {
            let cur_bbox = self.bbox();

            // If the clip path is a rect and completely covers the current bbox, don't emit it.
            if cur_bbox
                .min_x()
                .is_nearly_greater_or_equal(clip_rect.min_x())
                && cur_bbox
                    .min_y()
                    .is_nearly_greater_or_equal(clip_rect.min_y())
                && cur_bbox.max_x().is_nearly_less_or_equal(clip_rect.max_x())
                && cur_bbox.max_y().is_nearly_less_or_equal(clip_rect.max_y())
            {
                self.get_mut().clips.push(ClipType::Dummy);
                return;
            }
        }

        let bbox = clip_path.bounding_box();
        device.push_clip_path(&ClipPath {
            path: clip_path,
            fill,
        });
        self.push_bbox(bbox);
        self.get_mut().clips.push(ClipType::Real);
    }

    pub(crate) fn pop_clip_path(&mut self, device: &mut impl Device<'a>) {
        if let Some(ClipType::Real) = self.get_mut().clips.pop() {
            device.pop_clip_path();
            self.pop_bbox();
        }
    }

    fn pop_bbox(&mut self) {
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

    pub(crate) fn restore_state(&mut self, device: &mut impl Device<'a>) {
        let Some(target_clips) = self
            .states
            .get(self.states.len().saturating_sub(2))
            .map(|s| s.clips.len())
        else {
            warn!("underflowed graphics state");
            return;
        };

        while self.get().clips.len() > target_clips {
            self.pop_clip_path(device);
        }

        // The first state should never be popped.
        if self.states.len() > 1 {
            self.states.pop();
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
        self.pre_concat_affine(convert_transform(transform));
    }

    pub(crate) fn pre_concat_affine(&mut self, transform: Affine) {
        self.get_mut().ctm *= transform;
    }

    pub(crate) fn get_font(
        &mut self,
        resources: &Resources<'a>,
        name: Name<'_>,
    ) -> Option<Font<'a>> {
        resources.get_font(
            name,
            Box::new(|ref_| {
                self.font_cache
                    .entry(ref_)
                    .or_insert_with(|| {
                        resources
                            .resolve_ref::<Dict<'_>>(ref_)
                            .and_then(|o| Font::new(&o, &self.settings.font_resolver))
                    })
                    .clone()
            }),
            Box::new(|c| Font::new(&c, &self.settings.font_resolver)),
        )
    }

    pub(crate) fn get_color_space(
        &mut self,
        resources: &Resources<'_>,
        name: Name<'_>,
    ) -> Option<ColorSpace> {
        resources.get_color_space(
            name,
            Box::new(|ref_| {
                self.object_cache.get_or_insert_with(ref_.into(), || {
                    resources
                        .resolve_ref::<Object<'_>>(ref_)
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

fn path_as_rect(path: &BezPath) -> Option<Rect> {
    let bbox = path.bounding_box();
    let (min_x, min_y, max_x, max_y) = (bbox.min_x(), bbox.min_y(), bbox.max_x(), bbox.max_y());
    let mut touched = [false; 4];

    // One MoveTo, three LineTo, one ClosePath
    if path.elements().len() != 5 {
        return None;
    }

    let mut check_point = |p: Point| {
        touched[0] |= p.x.is_nearly_equal(min_x);
        touched[1] |= p.y.is_nearly_equal(min_y);
        touched[2] |= p.x.is_nearly_equal(max_x);
        touched[3] |= p.y.is_nearly_equal(max_y);
    };

    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => check_point(*p),
            PathEl::LineTo(l) => check_point(*l),
            PathEl::QuadTo(_, _) => {
                return None;
            }
            PathEl::CurveTo(_, _, _) => {
                return None;
            }
            PathEl::ClosePath => {}
        }
    }

    if touched[0] && touched[1] && touched[2] && touched[3] {
        Some(bbox)
    } else {
        None
    }
}
