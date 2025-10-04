use crate::GlyphDrawMode;
use crate::context::Context;
use crate::device::Device;
use crate::font::Glyph;
use crate::interpret::path::get_paint;
use hayro_syntax::object;
use hayro_syntax::page::Resources;
use kurbo::Affine;
use log::warn;

pub(crate) fn show_text_string<'a>(
    ctx: &mut Context<'a>,
    device: &mut impl Device<'a>,
    resources: &Resources<'a>,
    text: object::String,
) {
    let Some(font) = ctx.get().text_state.font.clone() else {
        warn!("tried to show text without active font");

        return;
    };

    let text_str = text.get();
    let bytes = text_str.as_ref();
    let mut cur_idx = 0;

    while cur_idx < bytes.len() {
        let (code, adv) = font.read_code(bytes, cur_idx);
        cur_idx += adv;

        let (glyph, glyph_transform) = font.get_glyph(
            font.map_code(code),
            ctx,
            resources,
            font.origin_displacement(code),
        );
        show_glyph(ctx, device, &glyph, glyph_transform);

        ctx.get_mut().text_state.apply_code_advance(code, adv);
    }
}

pub(crate) fn next_line(ctx: &mut Context, tx: f64, ty: f64) {
    let new_matrix = ctx.get_mut().text_state.text_line_matrix * Affine::translate((tx, ty));
    ctx.get_mut().text_state.text_line_matrix = new_matrix;
    ctx.get_mut().text_state.text_matrix = new_matrix;
}

pub(crate) fn show_glyph<'a>(
    ctx: &mut Context<'a>,
    device: &mut impl Device<'a>,
    glyph: &Glyph<'a>,
    glyph_transform: Affine,
) {
    if !ctx.ocg_state.is_visible() {
        return;
    }

    device.set_soft_mask(ctx.get().graphics_state.soft_mask.clone());
    device.set_blend_mode(ctx.get().graphics_state.blend_mode);
    let stroke_props = ctx.stroke_props();

    match ctx.get().text_state.render_mode {
        TextRenderingMode::Fill => {
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, false),
                &GlyphDrawMode::Fill,
            );
        }
        TextRenderingMode::Stroke => {
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, true),
                &GlyphDrawMode::Stroke(stroke_props),
            );
        }
        TextRenderingMode::FillStroke => {
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, false),
                &GlyphDrawMode::Fill,
            );
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, true),
                &GlyphDrawMode::Stroke(stroke_props),
            );
        }
        TextRenderingMode::Invisible => {}
        TextRenderingMode::Clip => {
            clip_glyph(ctx, glyph, glyph_transform);
        }
        TextRenderingMode::FillAndClip => {
            clip_glyph(ctx, glyph, glyph_transform);
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, false),
                &GlyphDrawMode::Fill,
            );
        }
        TextRenderingMode::StrokeAndClip => {
            clip_glyph(ctx, glyph, glyph_transform);
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, true),
                &GlyphDrawMode::Stroke(stroke_props),
            );
        }
        TextRenderingMode::FillAndStrokeAndClip => {
            clip_glyph(ctx, glyph, glyph_transform);
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, false),
                &GlyphDrawMode::Fill,
            );
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, true),
                &GlyphDrawMode::Stroke(stroke_props),
            );
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
