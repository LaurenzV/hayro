use crate::renderer::SvgRenderer;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::{Context, InterpreterSettings, interpret_page};
use kurbo::Rect;
use std::fmt;
use std::fmt::{Display, Formatter};

pub(crate) mod image;
mod renderer;

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
