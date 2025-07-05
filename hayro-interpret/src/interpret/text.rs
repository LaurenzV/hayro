use crate::context::Context;
use crate::device::Device;
use crate::font::Glyph;
use crate::interpret::path::get_paint;
use hayro_syntax::document::page::Resources;
use hayro_syntax::object::string;
use kurbo::Affine;
use log::warn;

pub(crate) fn show_text_string<'a>(
    ctx: &mut Context<'a>,
    device: &mut impl Device,
    resources: &Resources<'a>,
    text: string::String,
) {
    let Some(font) = ctx.get().text_state.font.clone() else {
        warn!("tried to show text without active font");

        return;
    };

    let code_len = font.code_len();
    for b in text.get().chunks(code_len) {
        let code = match code_len {
            1 => b[0] as u16,
            2 => u16::from_be_bytes([b[0], b[1]]),
            _ => unimplemented!(),
        };

        let glyph = font.get_glyph(
            font.map_code(code),
            ctx,
            resources,
            font.origin_displacement(code),
        );
        show_glyph(ctx, device, &glyph);

        ctx.get_mut().text_state.apply_code_advance(code);
    }
}

pub(crate) fn next_line(ctx: &mut Context, tx: f64, ty: f64) {
    let new_matrix = ctx.get_mut().text_state.text_line_matrix * Affine::translate((tx, ty));
    ctx.get_mut().text_state.text_line_matrix = new_matrix;
    ctx.get_mut().text_state.text_matrix = new_matrix;
}

pub(crate) fn show_glyph<'a>(ctx: &mut Context<'a>, device: &mut impl Device, glyph: &Glyph<'a>) {
    device.set_transform(ctx.get().ctm);

    device.set_soft_mask(ctx.get().soft_mask.clone());
    device.set_stroke_properties(&ctx.stroke_props());
    device.set_fill_properties(&ctx.fill_props());

    match ctx.get().text_state.render_mode {
        TextRenderingMode::Fill => {
            device.fill_glyph(glyph, &get_paint(ctx, false));
        }
        TextRenderingMode::Stroke => {
            device.stroke_glyph(glyph, &get_paint(ctx, true));
        }
        TextRenderingMode::FillStroke => {
            device.fill_glyph(glyph, &get_paint(ctx, false));
            device.stroke_glyph(glyph, &get_paint(ctx, true));
        }
        TextRenderingMode::Invisible => {}
        TextRenderingMode::Clip => {
            clip_glyph(ctx, glyph, glyph.glyph_transform());
        }
        TextRenderingMode::FillAndClip => {
            clip_glyph(ctx, glyph, glyph.glyph_transform());
            device.fill_glyph(glyph, &get_paint(ctx, false));
        }
        TextRenderingMode::StrokeAndClip => {
            clip_glyph(ctx, glyph, glyph.glyph_transform());
            device.stroke_glyph(glyph, &get_paint(ctx, true));
        }
        TextRenderingMode::FillAndStrokeAndClip => {
            clip_glyph(ctx, glyph, glyph.glyph_transform());
            device.fill_glyph(glyph, &get_paint(ctx, false));
            device.stroke_glyph(glyph, &get_paint(ctx, true));
        }
    }
}

pub(crate) fn clip_glyph(context: &mut Context, glyph: &Glyph, transform: Affine) {
    match glyph {
        Glyph::Outline(o) => {
            let outline = transform * o.outline();
            let has_outline = outline.segments().next().is_some();

            if has_outline {
                context.get_mut().text_state.clip_paths.extend(outline);
            }
        }
        Glyph::Type3(_) => {
            warn!("text rendering mode clip not implemented for shape glyphs");
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum TextRenderingMode {
    #[default]
    Fill,
    Stroke,
    FillStroke,
    Invisible,
    FillAndClip,
    StrokeAndClip,
    FillAndStrokeAndClip,
    Clip,
}
