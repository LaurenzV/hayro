use crate::color::ColorSpace;
use crate::context::Context;
use crate::convert::{convert_line_cap, convert_line_join};
use crate::device::Device;
use crate::font::{FontData, FontQuery};
use crate::interpret::path::{
    close_path, fill_path, fill_path_impl, fill_stroke_path, stroke_path,
};
use crate::interpret::state::handle_gs;
use crate::interpret::text::TextRenderingMode;
use crate::pattern::{Pattern, ShadingPattern};
use crate::shading::Shading;
use crate::util::OptionLog;
use crate::x_object::{ImageXObject, XObject, draw_image_xobject, draw_xobject};
use crate::{BlendMode, FillRule};
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::OC;
use hayro_syntax::object::{Dict, Object, dict_or_stream};
use hayro_syntax::page::{Page, Resources};
use kurbo::{Affine, Point, Shape};
use log::warn;
use smallvec::smallvec;
use std::sync::Arc;

pub(crate) mod path;
pub(crate) mod state;
pub(crate) mod text;

/// A callback function for resolving font queries.
///
/// The first argument is the raw data, the second argument is the index in case the font
/// is a TTC, otherwise it should be 0.
pub type FontResolverFn = Arc<dyn Fn(&FontQuery) -> Option<(FontData, u32)> + Send + Sync>;
/// A callback function for resolving warnings during interpretation.
pub type WarningSinkFn = Arc<dyn Fn(InterpreterWarning) + Send + Sync>;

#[derive(Clone)]
/// Settings that should be applied during the interpretation process.
pub struct InterpreterSettings {
    /// Nearly every PDF contains text. In most cases, PDF files embed the fonts they use, and
    /// hayro can therefore read the font files and do all the processing needed. However, there
    /// are two problems:
    /// - Fonts don't _have_ to be embedded, it's possible that the PDF file only defines the basic
    ///   metadata of the font, like its name, but relies on the PDF processor to find that font
    ///   in its environment.
    /// - The PDF specification requires a list of 14 fonts that should always be available to a
    ///   PDF processor. These include:
    ///   - Times New Roman (Normal, Bold, Italic, BoldItalic)
    ///   - Courier (Normal, Bold, Italic, BoldItalic)
    ///   - Helvetica (Normal, Bold, Italic, BoldItalic)
    ///   - ZapfDingBats
    ///   - Symbol
    ///
    /// Because of this, if any of the above situations occurs, this callback will be called, which
    /// expects the data of an appropriate font to be returned, if available. If no such font is
    /// provided, the text will most likely fail to render.
    ///
    /// For the font data, there are two different formats that are accepted:
    /// - Any valid TTF/OTF font.
    /// - A valid CFF font program.
    ///
    /// The following recommendations are given for the implementation of this callback function.
    ///
    /// For the standard fonts, in case the original fonts are available on the system, you should
    /// just return those. Otherwise, for Helvetica, Courier and Times New Roman, the best alternative
    /// are the corresponding fonts of the [Liberation font family](https://github.com/liberationfonts/liberation-fonts).
    /// If you prefer smaller fonts, you can use the [Foxit CFF fonts](https://github.com/LaurenzV/hayro/tree/master/assets/standard_fonts),
    /// which are much smaller but are missing glyphs for certain scripts.
    ///
    /// For the `Symbol` and `ZapfDingBats` fonts, you should also prefer the system fonts, and if
    /// not available to you, you can, similarly to above, use the corresponding fonts from Foxit.
    ///
    /// If you don't want having to deal with this, you can just enable the `embed-fonts` feature
    /// and use the default implementation of the callback.
    pub font_resolver: FontResolverFn,

    /// In certain cases, `hayro` will emit a warning in case an issue was encountered while interpreting
    /// the PDF file. Providing a callback allows you to catch those warnings and handle them, if desired.
    pub warning_sink: WarningSinkFn,
}

impl Default for InterpreterSettings {
    fn default() -> Self {
        Self {
            #[cfg(not(feature = "embed-fonts"))]
            font_resolver: Arc::new(|_| None),
            #[cfg(feature = "embed-fonts")]
            font_resolver: Arc::new(|query| match query {
                FontQuery::Standard(s) => Some(s.get_font_data()),
                FontQuery::Fallback(f) => Some(f.pick_standard_font().get_font_data()),
            }),
            warning_sink: Arc::new(|_| {}),
        }
    }
}

#[derive(Copy, Clone, Debug)]
/// Warnings that can occur while interpreting a PDF file.
pub enum InterpreterWarning {
    /// A JPX image was encountered, even though the `jpeg2000` feature is not enabled.
    JpxImage,
    /// An unsupported font kind was encountered.
    ///
    /// Currently, only CID fonts with non-identity encoding are unsupported.
    UnsupportedFont,
    /// An image failed to decode.
    ImageDecodeFailure,
}

/// interpret the contents of the page and render them into the device.
pub fn interpret_page<'a>(
    page: &Page<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    let resources = page.resources();
    interpret(page.typed_operations(), resources, context, device)
}

/// Interpret the instructions from `ops` and render them into the device.
pub fn interpret<'a, 'b>(
    ops: impl Iterator<Item = TypedInstruction<'b>>,
    resources: &Resources<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    let num_states = context.num_states();

    context.save_state();

    for op in ops {
        match op {
            TypedInstruction::SaveState(_) => context.save_state(),
            TypedInstruction::StrokeColorDeviceRgb(s) => {
                context.get_mut().graphics_state.stroke_cs = ColorSpace::device_rgb();
                context.get_mut().graphics_state.stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32()];
            }
            TypedInstruction::StrokeColorDeviceGray(s) => {
                context.get_mut().graphics_state.stroke_cs = ColorSpace::device_gray();
                context.get_mut().graphics_state.stroke_color = smallvec![s.0.as_f32()];
            }
            TypedInstruction::StrokeColorCmyk(s) => {
                context.get_mut().graphics_state.stroke_cs = ColorSpace::device_cmyk();
                context.get_mut().graphics_state.stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32(), s.3.as_f32()];
            }
            TypedInstruction::LineWidth(w) => {
                context.get_mut().graphics_state.stroke_props.line_width = w.0.as_f32();
            }
            TypedInstruction::LineCap(c) => {
                context.get_mut().graphics_state.stroke_props.line_cap = convert_line_cap(c);
            }
            TypedInstruction::LineJoin(j) => {
                context.get_mut().graphics_state.stroke_props.line_join = convert_line_join(j);
            }
            TypedInstruction::MiterLimit(l) => {
                context.get_mut().graphics_state.stroke_props.miter_limit = l.0.as_f32();
            }
            TypedInstruction::Transform(t) => {
                context.pre_concat_transform(t);
            }
            TypedInstruction::RectPath(r) => {
                let rect = kurbo::Rect::new(
                    r.0.as_f64(),
                    r.1.as_f64(),
                    r.0.as_f64() + r.2.as_f64(),
                    r.1.as_f64() + r.3.as_f64(),
                )
                .to_path(0.1);
                context.path_mut().extend(rect);
            }
            TypedInstruction::MoveTo(m) => {
                let p = Point::new(m.0.as_f64(), m.1.as_f64());
                *(context.last_point_mut()) = p;
                *(context.sub_path_start_mut()) = p;
                context.path_mut().move_to(p);
            }
            TypedInstruction::FillPathEvenOdd(_) => {
                fill_path(context, device, FillRule::EvenOdd);
            }
            TypedInstruction::FillPathNonZero(_) => {
                fill_path(context, device, FillRule::NonZero);
            }
            TypedInstruction::FillPathNonZeroCompatibility(_) => {
                fill_path(context, device, FillRule::NonZero);
            }
            TypedInstruction::FillAndStrokeEvenOdd(_) => {
                fill_stroke_path(context, device, FillRule::EvenOdd);
            }
            TypedInstruction::FillAndStrokeNonZero(_) => {
                fill_stroke_path(context, device, FillRule::NonZero);
            }
            TypedInstruction::CloseAndStrokePath(_) => {
                close_path(context);
                stroke_path(context, device);
            }
            TypedInstruction::CloseFillAndStrokeEvenOdd(_) => {
                close_path(context);
                fill_stroke_path(context, device, FillRule::EvenOdd);
            }
            TypedInstruction::CloseFillAndStrokeNonZero(_) => {
                close_path(context);
                fill_stroke_path(context, device, FillRule::NonZero);
            }
            TypedInstruction::NonStrokeColorDeviceGray(s) => {
                context.get_mut().graphics_state.none_stroke_cs = ColorSpace::device_gray();
                context.get_mut().graphics_state.non_stroke_color = smallvec![s.0.as_f32()];
            }
            TypedInstruction::NonStrokeColorDeviceRgb(s) => {
                context.get_mut().graphics_state.none_stroke_cs = ColorSpace::device_rgb();
                context.get_mut().graphics_state.non_stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32()];
            }
            TypedInstruction::NonStrokeColorCmyk(s) => {
                context.get_mut().graphics_state.none_stroke_cs = ColorSpace::device_cmyk();
                context.get_mut().graphics_state.non_stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32(), s.3.as_f32()];
            }
            TypedInstruction::LineTo(m) => {
                if !context.path().elements().is_empty() {
                    let last_point = *context.last_point();
                    let mut p = Point::new(m.0.as_f64(), m.1.as_f64());
                    *(context.last_point_mut()) = p;
                    if last_point == p {
                        // Add a small delta so that zero width lines can still have a round stroke.
                        p.x += 0.0001;
                    }

                    context.path_mut().line_to(p);
                }
            }
            TypedInstruction::CubicTo(c) => {
                if !context.path().elements().is_empty() {
                    let p1 = Point::new(c.0.as_f64(), c.1.as_f64());
                    let p2 = Point::new(c.2.as_f64(), c.3.as_f64());
                    let p3 = Point::new(c.4.as_f64(), c.5.as_f64());

                    *(context.last_point_mut()) = p3;

                    context.path_mut().curve_to(p1, p2, p3)
                }
            }
            TypedInstruction::CubicStartTo(c) => {
                if !context.path().elements().is_empty() {
                    let p1 = *context.last_point();
                    let p2 = Point::new(c.0.as_f64(), c.1.as_f64());
                    let p3 = Point::new(c.2.as_f64(), c.3.as_f64());

                    *(context.last_point_mut()) = p3;

                    context.path_mut().curve_to(p1, p2, p3)
                }
            }
            TypedInstruction::CubicEndTo(c) => {
                if !context.path().elements().is_empty() {
                    let p2 = Point::new(c.0.as_f64(), c.1.as_f64());
                    let p3 = Point::new(c.2.as_f64(), c.3.as_f64());

                    *(context.last_point_mut()) = p3;

                    context.path_mut().curve_to(p2, p3, p3)
                }
            }
            TypedInstruction::ClosePath(_) => {
                close_path(context);
            }
            TypedInstruction::SetGraphicsState(gs) => {
                if let Some(gs) = resources
                    .get_ext_g_state::<Dict>(gs.0.clone(), Box::new(|_| None), Box::new(Some))
                    .warn_none(&format!("failed to get extgstate {}", gs.0.as_str()))
                {
                    handle_gs(&gs, context, resources);
                }
            }
            TypedInstruction::StrokePath(_) => {
                stroke_path(context, device);
            }
            TypedInstruction::EndPath(_) => {
                if let Some(clip) = *context.clip()
                    && !context.path().elements().is_empty()
                {
                    let clip_path = context.get().ctm * context.path().clone();
                    context.push_clip_path(clip_path, clip, device);

                    *(context.clip_mut()) = None;
                }

                context.path_mut().truncate(0);
            }
            TypedInstruction::NonStrokeColor(c) => {
                let fill_c = &mut context.get_mut().graphics_state.non_stroke_color;
                fill_c.truncate(0);

                for e in c.0 {
                    fill_c.push(e.as_f32());
                }
            }
            TypedInstruction::StrokeColor(c) => {
                let stroke_c = &mut context.get_mut().graphics_state.stroke_color;
                stroke_c.truncate(0);

                for e in c.0 {
                    stroke_c.push(e.as_f32());
                }
            }
            TypedInstruction::ClipNonZero(_) => {
                *(context.clip_mut()) = Some(FillRule::NonZero);
            }
            TypedInstruction::ClipEvenOdd(_) => {
                *(context.clip_mut()) = Some(FillRule::EvenOdd);
            }
            TypedInstruction::RestoreState(_) => context.restore_state(device),
            TypedInstruction::FlatnessTolerance(_) => {
                // Ignore for now.
            }
            TypedInstruction::ColorSpaceStroke(c) => {
                let cs = if let Some(named) = ColorSpace::new_from_name(c.0.clone()) {
                    named
                } else {
                    context
                        .get_color_space(resources, c.0)
                        .unwrap_or(ColorSpace::device_gray())
                };

                context.get_mut().graphics_state.stroke_color = cs.initial_color();
                context.get_mut().graphics_state.stroke_cs = cs;
            }
            TypedInstruction::ColorSpaceNonStroke(c) => {
                let cs = if let Some(named) = ColorSpace::new_from_name(c.0.clone()) {
                    named
                } else {
                    context
                        .get_color_space(resources, c.0)
                        .unwrap_or(ColorSpace::device_gray())
                };

                context.get_mut().graphics_state.non_stroke_color = cs.initial_color();
                context.get_mut().graphics_state.none_stroke_cs = cs;
            }
            TypedInstruction::DashPattern(p) => {
                context.get_mut().graphics_state.stroke_props.dash_offset = p.1.as_f32();
                // kurbo apparently cannot properly deal with offsets that are exactly 0.
                context.get_mut().graphics_state.stroke_props.dash_array =
                    p.0.iter::<f32>()
                        .map(|n| if n == 0.0 { 0.01 } else { n })
                        .collect();
            }
            TypedInstruction::RenderingIntent(_) => {
                // Ignore for now.
            }
            TypedInstruction::NonStrokeColorNamed(n) => {
                context.get_mut().graphics_state.non_stroke_color =
                    n.0.into_iter().map(|n| n.as_f32()).collect();
                context.get_mut().graphics_state.non_stroke_pattern = n.1.and_then(|name| {
                    resources.get_pattern(
                        name,
                        Box::new(|_| None),
                        Box::new(|d| Pattern::new(d, context, resources)),
                    )
                });
            }
            TypedInstruction::StrokeColorNamed(n) => {
                context.get_mut().graphics_state.stroke_color =
                    n.0.into_iter().map(|n| n.as_f32()).collect();
                context.get_mut().graphics_state.stroke_pattern = n.1.and_then(|name| {
                    resources.get_pattern(
                        name,
                        Box::new(|_| None),
                        Box::new(|d| Pattern::new(d, context, resources)),
                    )
                });
            }
            TypedInstruction::BeginMarkedContentWithProperties(bdc) => {
                // Properties can be either:
                // 1. A Name that references an entry in the Resources/Properties dictionary
                // 2. An inline dictionary with an OC key

                if let Some(name) = bdc.1.clone().into_name()
                    && let Some(ocg_ref) = resources.properties.get_ref(name.clone())
                {
                    context.ocg_state.begin_ocg(ocg_ref.into());
                } else if let Some((props, _)) = dict_or_stream(&bdc.1)
                    && let Some(oc_ref) = props.get_ref(OC)
                {
                    context.ocg_state.begin_ocg(oc_ref.into());
                } else {
                    context.ocg_state.begin_marked_content();
                }
            }
            TypedInstruction::MarkedContentPointWithProperties(_) => {}
            TypedInstruction::EndMarkedContent(_) => {
                context.ocg_state.end_marked_content();
            }
            TypedInstruction::MarkedContentPoint(_) => {}
            TypedInstruction::BeginMarkedContent(_) => {
                context.ocg_state.begin_marked_content();
            }
            TypedInstruction::BeginText(_) => {
                context.get_mut().text_state.text_matrix = Affine::IDENTITY;
                context.get_mut().text_state.text_line_matrix = Affine::IDENTITY;
            }
            TypedInstruction::SetTextMatrix(m) => {
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
            TypedInstruction::EndText(_) => {
                let has_outline = context
                    .get()
                    .text_state
                    .clip_paths
                    .segments()
                    .next()
                    .is_some();

                if has_outline {
                    let clip_path = context.get().ctm * context.get().text_state.clip_paths.clone();

                    context.push_clip_path(clip_path, FillRule::NonZero, device);
                }

                context.get_mut().text_state.clip_paths.truncate(0);
            }
            TypedInstruction::TextFont(t) => {
                let font = context.get_font(resources, t.0);
                context.get_mut().text_state.font_size = t.1.as_f32();
                context.get_mut().text_state.font = font;
            }
            TypedInstruction::ShowText(s) => {
                text::show_text_string(context, device, resources, s.0);
            }
            TypedInstruction::ShowTexts(s) => {
                for obj in s.0.iter::<Object>() {
                    if let Some(adjustment) = obj.clone().into_f32() {
                        context.get_mut().text_state.apply_adjustment(adjustment);
                    } else if let Some(text) = obj.into_string() {
                        text::show_text_string(context, device, resources, text);
                    }
                }
            }
            TypedInstruction::HorizontalScaling(h) => {
                context.get_mut().text_state.horizontal_scaling = h.0.as_f32();
            }
            TypedInstruction::TextLeading(tl) => {
                context.get_mut().text_state.leading = tl.0.as_f32();
            }
            TypedInstruction::CharacterSpacing(c) => {
                context.get_mut().text_state.char_space = c.0.as_f32()
            }
            TypedInstruction::WordSpacing(w) => {
                context.get_mut().text_state.word_space = w.0.as_f32();
            }
            TypedInstruction::NextLine(n) => {
                let (tx, ty) = (n.0.as_f64(), n.1.as_f64());
                text::next_line(context, tx, ty)
            }
            TypedInstruction::NextLineUsingLeading(_) => {
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
            }
            TypedInstruction::NextLineAndShowText(n) => {
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
                text::show_text_string(context, device, resources, n.0)
            }
            TypedInstruction::TextRenderingMode(r) => {
                let mode = match r.0.as_i64() {
                    0 => TextRenderingMode::Fill,
                    1 => TextRenderingMode::Stroke,
                    2 => TextRenderingMode::FillStroke,
                    3 => TextRenderingMode::Invisible,
                    4 => TextRenderingMode::FillAndClip,
                    5 => TextRenderingMode::StrokeAndClip,
                    6 => TextRenderingMode::FillAndStrokeAndClip,
                    7 => TextRenderingMode::Clip,
                    _ => {
                        warn!("unknown text rendering mode {}", r.0.as_i64());

                        TextRenderingMode::Fill
                    }
                };

                context.get_mut().text_state.render_mode = mode;
            }
            TypedInstruction::NextLineAndSetLeading(n) => {
                let (tx, ty) = (n.0.as_f64(), n.1.as_f64());
                context.get_mut().text_state.leading = -ty as f32;
                text::next_line(context, tx, ty)
            }
            TypedInstruction::ShapeGlyph(_) => {}
            TypedInstruction::XObject(x) => {
                let cache = context.object_cache.clone();
                let transfer_function = context.get().graphics_state.transfer_function.clone();
                if let Some(x_object) = resources.get_x_object(
                    x.0,
                    Box::new(|_| None),
                    Box::new(|s| {
                        XObject::new(
                            &s,
                            &context.settings.warning_sink,
                            &cache,
                            transfer_function.clone(),
                        )
                    }),
                ) {
                    draw_xobject(&x_object, resources, context, device);
                }
            }
            TypedInstruction::InlineImage(i) => {
                let warning_sink = context.settings.warning_sink.clone();
                let transfer_function = context.get().graphics_state.transfer_function.clone();
                let cache = context.object_cache.clone();
                if let Some(x_object) = ImageXObject::new(
                    &i.0,
                    |name| context.get_color_space(resources, name.clone()),
                    &warning_sink,
                    &cache,
                    false,
                    transfer_function,
                ) {
                    draw_image_xobject(&x_object, context, device);
                }
            }
            TypedInstruction::TextRise(t) => {
                context.get_mut().text_state.rise = t.0.as_f32();
            }
            TypedInstruction::Shading(s) => {
                if !context.ocg_state.is_visible() {
                    continue;
                }

                if let Some(sp) = resources
                    .get_shading(s.0, Box::new(|_| None), Box::new(Some))
                    .and_then(|o| dict_or_stream(&o))
                    .and_then(|s| Shading::new(&s.0, s.1.as_ref(), &context.object_cache))
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
                    st.graphics_state.non_stroke_pattern = Some(sp);
                    st.graphics_state.none_stroke_cs = ColorSpace::pattern();

                    device.set_soft_mask(st.graphics_state.soft_mask.clone());
                    device.set_blend_mode(st.graphics_state.blend_mode);
                    device.push_transparency_group(
                        st.graphics_state.non_stroke_alpha,
                        None,
                        BlendMode::default(),
                    );

                    let bbox = context.bbox().to_path(0.1);
                    let inverted_bbox = context.get().ctm.inverse() * bbox;
                    fill_path_impl(context, device, FillRule::NonZero, Some(&inverted_bbox));

                    device.pop_transparency_group();

                    context.pop_root_transform();
                    context.restore_state(device);
                } else {
                    warn!("failed to process shading");
                }
            }
            TypedInstruction::BeginCompatibility(_) => {}
            TypedInstruction::EndCompatibility(_) => {}
            TypedInstruction::ColorGlyph(_) => {}
            TypedInstruction::ShowTextWithParameters(t) => {
                context.get_mut().text_state.word_space = t.0.as_f32();
                context.get_mut().text_state.char_space = t.1.as_f32();
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
                text::show_text_string(context, device, resources, t.2)
            }
            _ => {
                warn!("failed to read an operator");
            }
        }
    }

    while context.num_states() > num_states {
        context.restore_state(device);
    }
}
