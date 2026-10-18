//! The graphics state stack.

use crate::StrokeProps;
use crate::interpret::state::{ClipType, CompositingState, PaintState, State, TextState};
use kurbo::Affine;

/// A copy-on-write group of graphics state parameters.
struct Group<T> {
    // Sorted by depth, with at most one item per depth.
    items: Vec<(T, u16)>,
}

impl<T: Clone> Group<T> {
    fn new(value: T) -> Self {
        Self {
            items: vec![(value, 0)],
        }
    }

    fn get(&self, idx: u16) -> &T {
        &self.items[idx as usize].0
    }

    fn get_mut(&mut self, idx: &mut u16, depth: u16) -> &mut T {
        if self.items[*idx as usize].1 != depth {
            let value = self.items[*idx as usize].0.clone();
            self.items.push((value, depth));
            *idx = (self.items.len() - 1) as u16;
        }

        &mut self.items[*idx as usize].0
    }

    fn truncate(&mut self, idx: u16) {
        self.items.truncate(idx as usize + 1);
    }
}

#[derive(Clone, Copy)]
struct Frame {
    ctm: u16,
    stroke: u16,
    fill: u16,
    stroke_props: u16,
    text: u16,
    compositing: u16,
    clips: u32,
}

/// The graphics state stack.
pub(crate) struct StateStack<'a> {
    frames: Vec<Frame>,
    // Saves beyond the maximum depth.
    // TODO: Return an error instead once interpretation is fallible.
    overflow: usize,
    ctm: Group<Affine>,
    stroke: Group<PaintState<'a>>,
    fill: Group<PaintState<'a>>,
    stroke_props: Group<StrokeProps>,
    text: Group<TextState<'a>>,
    compositing: Group<CompositingState<'a>>,
    clips: Vec<ClipType>,
}

macro_rules! accessors {
    ($get:ident, $get_mut:ident, $field:ident, $ty:ty) => {
        pub(crate) fn $get(&self) -> &$ty {
            self.$field.get(self.top().$field)
        }

        pub(crate) fn $get_mut(&mut self) -> &mut $ty {
            let depth = self.depth();
            let frame = self.frames.last_mut().unwrap();
            self.$field.get_mut(&mut frame.$field, depth)
        }
    };
}

impl<'a> StateStack<'a> {
    pub(crate) fn new(state: State<'a>) -> Self {
        Self {
            frames: vec![Frame {
                ctm: 0,
                stroke: 0,
                fill: 0,
                stroke_props: 0,
                text: 0,
                compositing: 0,
                clips: 0,
            }],
            overflow: 0,
            ctm: Group::new(state.ctm),
            stroke: Group::new(state.stroke),
            fill: Group::new(state.fill),
            stroke_props: Group::new(state.stroke_props),
            text: Group::new(state.text_state),
            compositing: Group::new(state.compositing),
            clips: Vec::new(),
        }
    }

    fn top(&self) -> &Frame {
        self.frames.last().unwrap()
    }

    fn depth(&self) -> u16 {
        (self.frames.len() - 1) as u16
    }

    accessors!(ctm, ctm_mut, ctm, Affine);
    accessors!(stroke, stroke_mut, stroke, PaintState<'a>);
    accessors!(fill, fill_mut, fill, PaintState<'a>);
    accessors!(stroke_props, stroke_props_mut, stroke_props, StrokeProps);
    accessors!(text, text_mut, text, TextState<'a>);
    accessors!(
        compositing,
        compositing_mut,
        compositing,
        CompositingState<'a>
    );

    /// The number of states, including the initial one.
    pub(crate) fn len(&self) -> usize {
        self.frames.len() + self.overflow
    }

    pub(crate) fn save(&mut self) {
        if self.frames.len() > u16::MAX as usize {
            self.overflow += 1;
            return;
        }

        let mut frame = *self.top();
        frame.clips = self.clips.len() as u32;
        self.frames.push(frame);
    }

    /// The number of clips to keep when restoring, or `None` if there is nothing to restore.
    pub(crate) fn restore_target_clips(&self) -> Option<usize> {
        if self.overflow > 0 {
            return Some(self.clips.len());
        }

        (self.frames.len() > 1).then(|| self.top().clips as usize)
    }

    /// Restore the previous state. Excess clips must have been popped already.
    pub(crate) fn restore(&mut self) {
        if self.overflow > 0 {
            self.overflow -= 1;
            return;
        }

        if self.frames.len() <= 1 {
            return;
        }

        self.frames.pop();
        let top = *self.top();
        self.ctm.truncate(top.ctm);
        self.stroke.truncate(top.stroke);
        self.fill.truncate(top.fill);
        self.stroke_props.truncate(top.stroke_props);
        self.text.truncate(top.text);
        self.compositing.truncate(top.compositing);
    }

    pub(crate) fn push_clip(&mut self, clip: ClipType) {
        self.clips.push(clip);
    }

    pub(crate) fn pop_clip(&mut self) -> Option<ClipType> {
        self.clips.pop()
    }

    pub(crate) fn num_clips(&self) -> usize {
        self.clips.len()
    }

    /// A copy of the current state.
    pub(crate) fn snapshot(&self) -> State<'a> {
        State {
            ctm: *self.ctm(),
            stroke: self.stroke().clone(),
            fill: self.fill().clone(),
            stroke_props: self.stroke_props().clone(),
            text_state: self.text().clone(),
            compositing: self.compositing().clone(),
        }
    }
}
