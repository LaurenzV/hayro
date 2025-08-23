use std::collections::HashMap;
use std::fmt;
use std::fmt::{Display, Formatter};
use crate::renderer::SvgRenderer;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::{Context, InterpreterSettings, interpret_page};
use kurbo::Rect;

mod renderer;
pub(crate) mod image;

pub fn convert(page: &Page, interpreter_settings: &InterpreterSettings) -> String {
    let mut state = Context::new(
        page.initial_transform(true),
        Rect::new(
            0.0,
            0.0,
            page.render_dimensions().0 as f64,
            page.render_dimensions().1 as f64,
        ),
        page.xref(),
        interpreter_settings.clone(),
    );
    let mut device = SvgRenderer::new(page);
    device.write_header(page.render_dimensions());

    interpret_page(page, &mut state, &mut device);

    device.finish()
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct Id(char, u64);

impl Display for Id {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.0, self.1)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Deduplicator<T> {
    kind: char,
    vec: Vec<T>,
    present: HashMap<u128, Id>,
}

impl<T> Default for Deduplicator<T> {
    fn default() -> Self {
        Self::new('-')
    }
}

impl<T> Deduplicator<T> {
    fn new(kind: char) -> Self {
        Self {
            kind,
            vec: Vec::new(),
            present: HashMap::new(),
        }
    }

    fn insert_with<F>(&mut self, hash: u128, f: F) -> Id
    where
        F: FnOnce() -> T,
    {
        *self.present.entry(hash).or_insert_with(|| {
            let index = self.vec.len();
            self.vec.push(f());
            Id(self.kind, index as u64)
        })
    }

    fn iter(&self) -> impl Iterator<Item = (Id, &T)> {
        self.vec
            .iter()
            .enumerate()
            .map(|(i, v)| (Id(self.kind, i as u64), v))
    }

    fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }
}