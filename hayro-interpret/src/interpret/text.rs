use crate::GlyphDrawMode;
use crate::context::Context;
use crate::device::Device;
use crate::font::Glyph;
use crate::interpret::path::get_paint;
use crate::interpret::state::TextStateFont;
use crate::text_span::{GlyphPosition, TextSpan};
use hayro_cmap::BfString;
use hayro_syntax::object;
use hayro_syntax::page::Resources;
use kurbo::{Affine, Point};
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

    let bytes = text.as_bytes();

    // In case we have a fallback font (which occurs if either no font was set at all
    // in the content stream, or an invalid one), we only want to show the glyphs
    // using Helvetica if the bytes are actually valid ASCII.
    let show_glyphs = matches!(font, TextStateFont::Font(_))
        || (matches!(font, TextStateFont::Fallback(_)) && bytes.is_ascii());

    let font_size = ctx.get().text_state.font_size;
    let mut span_text = String::new();
    let mut span_glyphs = Vec::new();

    let mut cur_idx = 0;

    while cur_idx < bytes.len() {
        let (code, adv) = font.read_code(bytes, cur_idx);
        cur_idx += adv;

        if show_glyphs {
            // Capture the glyph position *before* drawing or advancing.
            let pos = (ctx.get().ctm * ctx.get().text_state.full_transform())
                * Point::new(0.0, 0.0);

            let (glyph, glyph_transform) = font.get_glyph(
                font.map_code(code),
                code,
                ctx,
                resources,
                font.origin_displacement(code),
            );

            // Resolve the Unicode mapping for this glyph.
            let mut glyph_text = String::new();
            if let Some(unicode) = glyph.as_unicode() {
                match unicode {
                    BfString::Char(c) => glyph_text.push(c),
                    BfString::String(s) => glyph_text.push_str(&s),
                }
            }

            show_glyph(ctx, device, &glyph, glyph_transform);
            ctx.get_mut().text_state.apply_code_advance(code, adv);

            // Compute advance from the position delta.
            let next_pos = (ctx.get().ctm * ctx.get().text_state.full_transform())
                * Point::new(0.0, 0.0);

            span_text.push_str(&glyph_text);
            span_glyphs.push(GlyphPosition {
                text: glyph_text,
                x: pos.x,
                y: pos.y,
                advance_x: next_pos.x - pos.x,
                advance_y: next_pos.y - pos.y,
                char_code: code,
            });
        } else {
            ctx.get_mut().text_state.apply_code_advance(code, adv);
        }
    }

    let effective = ctx.get().ctm * ctx.get().text_state.full_transform();
    let coeffs = effective.as_coeffs();
    let font_size_device = (coeffs[2] * coeffs[2] + coeffs[3] * coeffs[3]).sqrt() as f32;

    if !span_glyphs.is_empty() {
        device.draw_text_span(&TextSpan {
            text: span_text,
            glyphs: span_glyphs,
            font_size,
            font_size_device,
            tag: None,
            is_block_start: false,
            is_artifact: false,
        });
    }
}

pub(crate) fn next_line(ctx: &mut Context<'_>, tx: f64, ty: f64) {
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
        TextRenderingMode::Invisible => {
            // Still call draw_glyph for invisible text, so that it can
            // for example be used for text extraction.
            device.draw_glyph(
                glyph,
                ctx.get().ctm,
                glyph_transform,
                &get_paint(ctx, false),
                &GlyphDrawMode::Invisible,
            );
        }
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

pub(crate) fn clip_glyph(context: &mut Context<'_>, glyph: &Glyph<'_>, transform: Affine) {
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