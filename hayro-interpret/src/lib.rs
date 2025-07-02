use crate::convert::{convert_line_cap, convert_line_join};
use crate::device::Device;
use clip_path::ClipPath;
use hayro_syntax::content::ops::{LineCap, LineJoin, TypedOperation};
use hayro_syntax::document::page::Resources;
use hayro_syntax::object::dict::Dict;
use hayro_syntax::object::name::Name;
use hayro_syntax::object::number::Number;
use hayro_syntax::object::{Object, dict_or_stream};
use kurbo::{Affine, Cap, Join, Point, Shape};
use log::warn;
use peniko::Fill;
use smallvec::{SmallVec, smallvec};
use std::sync::Arc;

pub mod cache;
pub mod clip_path;
pub mod color;
pub mod context;
mod convert;
pub mod device;
pub mod font;
mod image;
mod interpret;
pub mod mask;
mod paint;
pub mod pattern;
pub mod shading;
pub mod util;
pub mod x_object;

use crate::color::ColorSpace;
use crate::context::Context;
use crate::pattern::{Pattern, ShadingPattern};
use crate::shading::Shading;
use crate::util::OptionLog;
use crate::x_object::{ImageXObject, XObject, draw_image_xobject, draw_xobject};
use interpret::text::TextRenderingMode;

use crate::interpret::path::{fill_path, fill_path_impl, fill_stroke_path, stroke_path};
pub use image::{RgbaImage, StencilImage};
use interpret::text;
pub use paint::{Paint, PaintType};

#[derive(Clone, Debug)]
pub struct StrokeProps {
    pub line_width: f32,
    pub line_cap: Cap,
    pub line_join: Join,
    pub miter_limit: f32,
    pub dash_array: SmallVec<[f32; 4]>,
    pub dash_offset: f32,
}

#[derive(Clone, Debug)]
pub struct FillProps {
    pub fill_rule: Fill,
}

pub fn interpret<'a, 'b>(
    ops: impl Iterator<Item = TypedOperation<'b>>,
    resources: &Resources<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device,
) {
    let num_states = context.num_states();

    save_sate(context);

    for op in ops {
        match op {
            TypedOperation::SaveState(_) => save_sate(context),
            TypedOperation::StrokeColorDeviceRgb(s) => {
                context.get_mut().stroke_cs = ColorSpace::device_rgb();
                context.get_mut().stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32()];
            }
            TypedOperation::StrokeColorDeviceGray(s) => {
                context.get_mut().stroke_cs = ColorSpace::device_gray();
                context.get_mut().stroke_color = smallvec![s.0.as_f32()];
            }
            TypedOperation::StrokeColorCmyk(s) => {
                context.get_mut().stroke_cs = ColorSpace::device_cmyk();
                context.get_mut().stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32(), s.3.as_f32()];
            }
            TypedOperation::LineWidth(w) => {
                context.get_mut().line_width = w.0.as_f32();
            }
            TypedOperation::LineCap(c) => {
                context.get_mut().line_cap = convert_line_cap(c);
            }
            TypedOperation::LineJoin(j) => {
                context.get_mut().line_join = convert_line_join(j);
            }
            TypedOperation::MiterLimit(l) => {
                context.get_mut().miter_limit = l.0.as_f32();
            }
            TypedOperation::Transform(t) => {
                context.pre_concat_transform(t);
            }
            TypedOperation::RectPath(r) => {
                let rect = kurbo::Rect::new(
                    r.0.as_f64(),
                    r.1.as_f64(),
                    r.0.as_f64() + r.2.as_f64(),
                    r.1.as_f64() + r.3.as_f64(),
                )
                .to_path(0.1);
                context.path_mut().extend(rect);
            }
            TypedOperation::MoveTo(m) => {
                let p = Point::new(m.0.as_f64(), m.1.as_f64());
                *(context.last_point_mut()) = p;
                *(context.sub_path_start_mut()) = p;
                context.path_mut().move_to(p);
            }
            TypedOperation::FillPathEvenOdd(_) => {
                context.get_mut().fill_rule = Fill::EvenOdd;
                fill_path(context, device);
            }
            TypedOperation::FillPathNonZero(_) => {
                context.get_mut().fill_rule = Fill::NonZero;
                fill_path(context, device);
            }
            TypedOperation::FillPathNonZeroCompatibility(_) => {
                context.get_mut().fill_rule = Fill::NonZero;
                fill_path(context, device);
            }
            TypedOperation::FillAndStrokeEvenOdd(_) => {
                context.get_mut().fill_rule = Fill::EvenOdd;
                fill_stroke_path(context, device);
            }
            TypedOperation::FillAndStrokeNonZero(_) => {
                context.get_mut().fill_rule = Fill::NonZero;
                fill_stroke_path(context, device);
            }
            TypedOperation::CloseAndStrokePath(_) => {
                context.path_mut().close_path();
                stroke_path(context, device);
            }
            TypedOperation::CloseFillAndStrokeEvenOdd(_) => {
                context.path_mut().close_path();
                context.get_mut().fill_rule = Fill::EvenOdd;
                fill_stroke_path(context, device);
            }
            TypedOperation::CloseFillAndStrokeNonZero(_) => {
                context.path_mut().close_path();
                context.get_mut().fill_rule = Fill::NonZero;
                fill_stroke_path(context, device);
            }
            TypedOperation::NonStrokeColorDeviceGray(s) => {
                context.get_mut().none_stroke_cs = ColorSpace::device_gray();
                context.get_mut().non_stroke_color = smallvec![s.0.as_f32()];
            }
            TypedOperation::NonStrokeColorDeviceRgb(s) => {
                context.get_mut().none_stroke_cs = ColorSpace::device_rgb();
                context.get_mut().non_stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32()];
            }
            TypedOperation::NonStrokeColorCmyk(s) => {
                context.get_mut().none_stroke_cs = ColorSpace::device_cmyk();
                context.get_mut().non_stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32(), s.3.as_f32()];
            }
            TypedOperation::LineTo(m) => {
                let last_point = *context.last_point();
                let mut p = Point::new(m.0.as_f64(), m.1.as_f64());
                *(context.last_point_mut()) = p;
                if last_point == p {
                    // Add a small delta so that zero width lines can still have a round stroke.
                    p.x += 0.0001;
                }

                context.path_mut().line_to(p);
            }
            TypedOperation::CubicTo(c) => {
                let p1 = Point::new(c.0.as_f64(), c.1.as_f64());
                let p2 = Point::new(c.2.as_f64(), c.3.as_f64());
                let p3 = Point::new(c.4.as_f64(), c.5.as_f64());

                *(context.last_point_mut()) = p3;

                context.path_mut().curve_to(p1, p2, p3)
            }
            TypedOperation::CubicStartTo(c) => {
                let p1 = *context.last_point();
                let p2 = Point::new(c.0.as_f64(), c.1.as_f64());
                let p3 = Point::new(c.2.as_f64(), c.3.as_f64());

                *(context.last_point_mut()) = p3;

                context.path_mut().curve_to(p1, p2, p3)
            }
            TypedOperation::CubicEndTo(c) => {
                let p2 = Point::new(c.0.as_f64(), c.1.as_f64());
                let p3 = Point::new(c.2.as_f64(), c.3.as_f64());

                *(context.last_point_mut()) = p3;

                context.path_mut().curve_to(p2, p3, p3)
            }
            TypedOperation::ClosePath(_) => {
                context.path_mut().close_path();

                *(context.last_point_mut()) = *context.sub_path_start();
            }
            TypedOperation::SetGraphicsState(gs) => {
                if let Some(gs) =  resources
                    .get_ext_g_state::<Dict>(gs.0.clone(), Box::new(|_| None), Box::new(Some))
                    .warn_none(&format!("failed to get extgstate {}", gs.0.as_str())) {
                    handle_gs(&gs, context);
                }
            }
            TypedOperation::StrokePath(_) => {
                stroke_path(context, device);
            }
            TypedOperation::EndPath(_) => {
                if let Some(clip) = *context.clip() {
                    device.set_transform(context.get().ctm);
                    device.push_clip_path(&ClipPath {
                        path: context.path().clone(),
                        fill: clip,
                    });

                    *(context.clip_mut()) = None;
                    context.get_mut().n_clips += 1;
                }

                context.path_mut().truncate(0);
            }
            TypedOperation::NonStrokeColor(c) => {
                let fill_c = &mut context.get_mut().non_stroke_color;
                fill_c.truncate(0);

                for e in c.0 {
                    fill_c.push(e.as_f32());
                }
            }
            TypedOperation::StrokeColor(c) => {
                let stroke_c = &mut context.get_mut().stroke_color;
                stroke_c.truncate(0);

                for e in c.0 {
                    stroke_c.push(e.as_f32());
                }
            }
            TypedOperation::ClipNonZero(_) => {
                *(context.clip_mut()) = Some(Fill::NonZero);
            }
            TypedOperation::ClipEvenOdd(_) => {
                *(context.clip_mut()) = Some(Fill::EvenOdd);
            }
            TypedOperation::RestoreState(_) => restore_state(context, device),
            TypedOperation::FlatnessTolerance(_) => {
                // Ignore for now.
            }
            TypedOperation::ColorSpaceStroke(c) => {
                let cs = if let Some(named) = ColorSpace::new_from_name(c.0.clone()) {
                    named
                } else {
                    context
                        .get_color_space(resources, c.0)
                        .unwrap_or(ColorSpace::device_gray())
                };

                context.get_mut().stroke_color = cs.initial_color();
                context.get_mut().stroke_cs = cs;
            }
            TypedOperation::ColorSpaceNonStroke(c) => {
                let cs = if let Some(named) = ColorSpace::new_from_name(c.0.clone()) {
                    named
                } else {
                    context
                        .get_color_space(resources, c.0)
                        .unwrap_or(ColorSpace::device_gray())
                };

                context.get_mut().non_stroke_color = cs.initial_color();
                context.get_mut().none_stroke_cs = cs;
            }
            TypedOperation::DashPattern(p) => {
                context.get_mut().dash_offset = p.1.as_f32();
                // kurbo apparently cannot properly deal with offsets that are exactly 0.
                context.get_mut().dash_array =
                    p.0.iter::<f32>()
                        .map(|n| if n == 0.0 { 0.01 } else { n })
                        .collect();
            }
            TypedOperation::RenderingIntent(_) => {
                // Ignore for now.
            }
            TypedOperation::NonStrokeColorNamed(n) => {
                context.get_mut().non_stroke_color = n.0.into_iter().map(|n| n.as_f32()).collect();
                context.get_mut().non_stroke_pattern = n.1.and_then(|name| {
                    resources.get_pattern(
                        name,
                        Box::new(|_| None),
                        Box::new(|d| Pattern::new(d, context, resources)),
                    )
                });
            }
            TypedOperation::StrokeColorNamed(n) => {
                context.get_mut().stroke_color = n.0.into_iter().map(|n| n.as_f32()).collect();
                context.get_mut().stroke_pattern = n.1.and_then(|name| {
                    resources.get_pattern(
                        name,
                        Box::new(|_| None),
                        Box::new(|d| Pattern::new(d, context, resources)),
                    )
                });
            }
            TypedOperation::BeginMarkedContentWithProperties(_) => {}
            TypedOperation::MarkedContentPointWithProperties(_) => {}
            TypedOperation::EndMarkedContent(_) => {}
            TypedOperation::MarkedContentPoint(_) => {}
            TypedOperation::BeginMarkedContent(_) => {}
            TypedOperation::BeginText(_) => {
                context.get_mut().text_state.text_matrix = Affine::IDENTITY;
                context.get_mut().text_state.text_line_matrix = Affine::IDENTITY;
            }
            TypedOperation::SetTextMatrix(m) => {
                let m = Affine::new([
                    m.0.as_f64(),
                    m.1.as_f64(),
                    m.2.as_f64(),
                    m.3.as_f64(),
                    m.4.as_f64(),
                    m.5.as_f64(),
                ]);
                context.get_mut().text_state.text_line_matrix = m;
                context.get_mut().text_state.text_matrix = m;
            }
            TypedOperation::EndText(_) => {
                let has_outline = context
                    .get()
                    .text_state
                    .clip_paths
                    .segments()
                    .next()
                    .is_some();

                if has_outline {
                    device.set_transform(context.get().ctm);
                    device.push_clip_path(&ClipPath {
                        path: context.get().text_state.clip_paths.clone(),
                        fill: Fill::NonZero,
                    });
                    context.get_mut().n_clips += 1;
                }

                context.get_mut().text_state.clip_paths.truncate(0);
            }
            TypedOperation::TextFont(t) => {
                let font = context.get_font(resources, t.0);
                context.get_mut().text_state.font_size = t.1.as_f32();
                context.get_mut().text_state.font = font;
            }
            TypedOperation::ShowText(s) => {
                text::show_text_string(context, device, resources, s.0);
            }
            TypedOperation::ShowTexts(s) => {
                for obj in s.0.iter::<Object>() {
                    if let Some(adjustment) = obj.clone().into_f32() {
                        context.get_mut().text_state.apply_adjustment(adjustment);
                    } else if let Some(text) = obj.into_string() {
                        text::show_text_string(context, device, resources, text);
                    }
                }
            }
            TypedOperation::HorizontalScaling(h) => {
                context.get_mut().text_state.horizontal_scaling = h.0.as_f32();
            }
            TypedOperation::TextLeading(tl) => {
                context.get_mut().text_state.leading = tl.0.as_f32();
            }
            TypedOperation::CharacterSpacing(c) => {
                context.get_mut().text_state.char_space = c.0.as_f32()
            }
            TypedOperation::WordSpacing(w) => {
                context.get_mut().text_state.word_space = w.0.as_f32();
            }
            TypedOperation::NextLine(n) => {
                let (tx, ty) = (n.0.as_f64(), n.1.as_f64());
                text::next_line(context, tx, ty)
            }
            TypedOperation::NextLineUsingLeading(_) => {
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
            }
            TypedOperation::NextLineAndShowText(n) => {
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
                text::show_text_string(context, device, resources, n.0)
            }
            TypedOperation::TextRenderingMode(r) => {
                let mode = match r.0.as_i32() {
                    0 => TextRenderingMode::Fill,
                    1 => TextRenderingMode::Stroke,
                    2 => TextRenderingMode::FillStroke,
                    3 => TextRenderingMode::Invisible,
                    4 => TextRenderingMode::FillAndClip,
                    5 => TextRenderingMode::StrokeAndClip,
                    6 => TextRenderingMode::FillAndStrokeAndClip,
                    7 => TextRenderingMode::Clip,
                    _ => {
                        warn!("unknown text rendering mode {}", r.0.as_i32());

                        TextRenderingMode::Fill
                    }
                };

                context.get_mut().text_state.render_mode = mode;
            }
            TypedOperation::NextLineAndSetLeading(n) => {
                let (tx, ty) = (n.0.as_f64(), n.1.as_f64());
                context.get_mut().text_state.leading = -ty as f32;
                text::next_line(context, tx, ty)
            }
            TypedOperation::ShapeGlyph(_) => {}
            TypedOperation::XObject(x) => {
                if let Some(x_object) =
                    resources.get_x_object(x.0, Box::new(|_| None), Box::new(|s| XObject::new(&s)))
                {
                    draw_xobject(&x_object, resources, context, device);
                }
            }
            TypedOperation::InlineImage(i) => {
                if let Some(x_object) = ImageXObject::new(&i.0, |name| {
                    context.get_color_space(resources, name.clone())
                }) {
                    draw_image_xobject(&x_object, context, device);
                }
            }
            TypedOperation::TextRise(t) => {
                context.get_mut().text_state.rise = t.0.as_f32();
            }
            TypedOperation::Shading(s) => {
                if let Some(sp) = resources
                    .get_shading(s.0, Box::new(|_| None), Box::new(Some))
                    .and_then(|o| dict_or_stream(&o))
                    .and_then(|s| Shading::new(&s.0, s.1.as_ref()))
                    .map(|s| {
                        Pattern::Shading(ShadingPattern {
                            shading: Arc::new(s),
                            matrix: Affine::IDENTITY,
                        })
                    })
                {
                    context.save_state();
                    context.push_root_transform();
                    let st = context.get_mut();
                    st.non_stroke_pattern = Some(sp);
                    st.none_stroke_cs = ColorSpace::pattern();

                    device.push_transparency_group(st.non_stroke_alpha);

                    let bbox = context.bbox().to_path(0.1);
                    let inverted_bbox = context.get().ctm.inverse() * bbox;
                    fill_path_impl(context, device, Some(&inverted_bbox));

                    device.pop_transparency_group();

                    context.restore_state();
                } else {
                    warn!("failed to process shading");
                }
            }
            TypedOperation::BeginCompatibility(_) => {}
            TypedOperation::EndCompatibility(_) => {}
            TypedOperation::ColorGlyph(_) => {}
            TypedOperation::ShowTextWithParameters(t) => {
                context.get_mut().text_state.word_space = t.0.as_f32();
                context.get_mut().text_state.char_space = t.1.as_f32();
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
                text::show_text_string(context, device, resources, t.2)
            }
            _ => {
                warn!("Failed to read an operator");
            }
        }
    }

    while context.num_states() > num_states {
        restore_state(context, device);
    }
}

fn save_sate(ctx: &mut Context) {
    ctx.save_state();
}

fn restore_state(ctx: &mut Context, device: &mut impl Device) {
    let mut num_clips = ctx.get().n_clips;
    ctx.restore_state();
    let target_clips = ctx.get().n_clips;

    while num_clips > target_clips {
        device.pop_clip_path();
        num_clips -= 1;
    }
}

fn handle_gs(dict: &Dict, context: &mut Context) {
    for key in dict.keys() {
        handle_gs_single(dict, key.clone(), context).warn_none(&format!(
            "invalid value in graphics state for {}",
            key.as_str()
        ));
    }
}

fn handle_gs_single(dict: &Dict, key: Name, context: &mut Context) -> Option<()> {
    // TODO Can we use constants here somehow?
    match key.as_str() {
        "LW" => context.get_mut().line_width = dict.get::<f32>(key)?,
        "LC" => context.get_mut().line_cap = convert_line_cap(LineCap(dict.get::<Number>(key)?)),
        "LJ" => context.get_mut().line_join = convert_line_join(LineJoin(dict.get::<Number>(key)?)),
        "ML" => context.get_mut().miter_limit = dict.get::<f32>(key)?,
        "CA" => context.get_mut().stroke_alpha = dict.get::<f32>(key)?,
        "ca" => context.get_mut().non_stroke_alpha = dict.get::<f32>(key)?,
        "SMask" => {
            warn!("soft masks are not yet supported");
        }
        "Type" => {}
        _ => {}
    }

    Some(())
}
