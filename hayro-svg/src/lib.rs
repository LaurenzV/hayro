use crate::renderer::SvgRenderer;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::{Context, InterpreterSettings, interpret};
use kurbo::Rect;

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

    interpret(
        page.typed_operations(),
        page.resources(),
        &mut state,
        &mut device,
    );

    device.finish()
}
