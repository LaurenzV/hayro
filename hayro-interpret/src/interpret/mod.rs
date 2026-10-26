use crate::FillRule;
use crate::color::ColorSpace;
use crate::context::Context;
use crate::convert::{convert_line_cap, convert_line_join};
use crate::device::Device;
use crate::font::{Font, FontData, FontQuery, StandardFont};
use crate::interpret::path::{
    close_path, end_path, fill_path, fill_path_impl, fill_stroke_path, stroke_path,
};
use crate::interpret::state::{TextStateFont, handle_gs};
use crate::interpret::text::TextRenderingMode;
use crate::pattern::{Pattern, ShadingPattern};
use crate::shading::Shading;
use crate::util::RectExt;
use crate::x_object::{FormXObject, ImageXObject, XObject};
use bitflags::bitflags;
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::{ANNOTS, AP, AS, F, N, OC, RECT};
use hayro_syntax::object::{Array, Dict, Name, Object, Rect, Stream, dict_or_stream};
use hayro_syntax::page::{Page, Resources};
use kurbo::{Affine, Point, Shape};
use rustc_hash::FxHashMap;
use smallvec::smallvec;
use std::sync::Arc;

pub(crate) mod path;
pub(crate) mod stack;
pub(crate) mod state;
pub(crate) mod text;

const ANNOTATION_HIDDEN: u32 = 1 << 1;
const ANNOTATION_NO_VIEW: u32 = 1 << 5;

pub use state::ActiveTransferFunction;

/// A callback function for resolving font queries.
///
/// The first argument is the raw data, the second argument is the index in case the font
/// is a TTC, otherwise it should be 0.
pub type FontResolverFn = Arc<dyn Fn(&FontQuery) -> Option<(FontData, u32)> + Send + Sync>;
/// A callback function for resolving cmap names to their files.
pub type CMapResolverFn =
    Arc<dyn Fn(hayro_cmap::CMapName<'_>) -> Option<&'static [u8]> + Send + Sync>;
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
    ///   - Times New Roman (Normal, Bold, Italic, `BoldItalic`)
    ///   - Courier (Normal, Bold, Italic, `BoldItalic`)
    ///   - Helvetica (Normal, Bold, Italic, `BoldItalic`)
    ///   - `ZapfDingBats`
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
    /// A callback for resolving cmaps that aren't embedded.
    ///
    /// When the PDF requires using a cmap that is not directly embedded in the PDF,
    /// this callback will be called to attempt fetching the data of the file.
    ///
    /// When the `embed-cmaps` feature is enabled, this uses `load_embedded`
    /// method from `hayro-cmap` by default, which embeds the cmap files for
    /// all 61 predefined cmaps
    /// that the PDF specification requires to be readily available on a system.
    /// Otherwise, you can implement your custom logic for lazily fetching the
    /// data. If you are fine not supporting such PDFs, you can simply pass a closure
    /// that always returns `None`.
    pub cmap_resolver: CMapResolverFn,
    /// In certain cases, `hayro` will emit a warning in case an issue was encountered while interpreting
    /// the PDF file. Providing a callback allows you to catch those warnings and handle them, if desired.
    pub warning_sink: WarningSinkFn,
    /// Which parts of a page should be rendered.
    pub render_content: RenderContent,
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
            #[cfg(feature = "embed-cmaps")]
            cmap_resolver: Arc::new(hayro_cmap::load_embedded),
            #[cfg(not(feature = "embed-cmaps"))]
            cmap_resolver: Arc::new(|_| None),
            warning_sink: Arc::new(|_| {}),
            render_content: RenderContent::all(),
        }
    }
}

bitflags! {
    /// The parts of a page that should be rendered.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RenderContent: u8 {
        /// The content stream of the page.
        const PAGE = 1 << 0;
        /// The annotations of the page.
        const ANNOTATIONS = 1 << 1;
    }
}

#[derive(Copy, Clone, Debug)]
/// Warnings that can occur while interpreting a PDF file.
#[non_exhaustive]
pub enum InterpreterWarning {
    /// A font could not be loaded.
    UnsupportedFont,
    /// An image failed to decode.
    ImageDecodeFailure,
    /// An annotation has an `AP` dictionary, but its normal appearance could not
    /// be selected or loaded, so the annotation was not drawn.
    UnresolvedAnnotationAppearance,
}

/// interpret the contents of the page and render them into the device.
pub fn interpret_page<'a>(
    page: &Page<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    let resources = page.resources();
    let render_content = context.settings.render_content;

    if render_content.contains(RenderContent::PAGE) {
        interpret(page.typed_operations(), resources, context, device);
    }

    if render_content.contains(RenderContent::ANNOTATIONS)
        && let Some(annot_arr) = page.raw().get::<Array<'_>>(ANNOTS)
    {
        for annot in annot_arr.iter::<Dict<'_>>() {
            interpret_annotation(&annot, resources, context, device);
        }
    }
}

fn interpret_annotation<'a>(
    annot: &Dict<'a>,
    resources: &Resources<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    let flags = annot.get::<u32>(F).unwrap_or(0);

    if flags & (ANNOTATION_HIDDEN | ANNOTATION_NO_VIEW) != 0 {
        return;
    }

    if let Some(apx) = appearance_stream(annot).and_then(|o| FormXObject::new(&o)) {
        let Some(rect) = annot.get::<Rect>(RECT) else {
            return;
        };

        let annot_rect = rect.to_kurbo();
        // 12.5.5. Appearance streams
        // "The algorithm outlined in this subclause shall be used
        // to map from the coordinate system of the appearance XObject."

        // 1) The appearance’s bounding box (specified by its BBox entry)
        // shall be transformed, using Matrix, to produce a
        // quadrilateral with arbitrary orientation. The transformed
        // appearance box is the smallest upright rectangle that
        // encompasses this quadrilateral.
        let transformed_rect = (apx.matrix
            * kurbo::Rect::new(
                apx.bbox[0] as f64,
                apx.bbox[1] as f64,
                apx.bbox[2] as f64,
                apx.bbox[3] as f64,
            )
            .to_path(0.1))
        .bounding_box();

        // 2) A matrix A shall be computed that scales and translates
        // the transformed appearance box to align with the edges
        // of the annotation’s rectangle (specified by the Rect entry).
        // A maps the lower-left corner (the corner with the smallest
        // x and y coordinates) and the upper-right corner (the
        // corner with the greatest x and y coordinates) of the
        // transformed appearance box to the corresponding corners
        // of the annotation’s rectangle.
        let sx = annot_rect.width() / transformed_rect.width();
        let sy = annot_rect.height() / transformed_rect.height();
        let affine = Affine::new([
            sx,
            0.0,
            0.0,
            sy,
            annot_rect.x0 - sx * transformed_rect.x0,
            annot_rect.y0 - sy * transformed_rect.y0,
        ]);

        // 3) Matrix shall be concatenated with A to form a matrix
        // AA that maps from the appearance’s coordinate system to
        // the annotation’s rectangle in default user space.
        context.save_state();
        context.pre_concat_affine(affine);
        context.push_root_transform();

        apx.draw(resources, context, device);
        context.pop_root_transform();
        context.restore_state(device);
    } else if annot.get::<Dict<'_>>(AP).is_some() {
        (context.settings.warning_sink)(InterpreterWarning::UnresolvedAnnotationAppearance);
    }
}

fn appearance_stream<'a>(annot: &Dict<'a>) -> Option<Stream<'a>> {
    match annot.get::<Dict<'_>>(AP)?.get::<Object<'_>>(N)? {
        Object::Stream(stream) => Some(stream),
        Object::Dict(states) => annot
            .get::<Name<'_>>(AS)
            .and_then(|state| states.get::<Stream<'_>>(state))
            .or_else(|| states.get::<Stream<'_>>(b"Off")),
        _ => None,
    }
}

/// Interpret the instructions from `ops` and render them into the device.
pub fn interpret<'a>(
    mut ops: TypedIter<'_>,
    resources: &Resources<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    let num_states = context.num_states();
    let mut font_dict_cache = FxHashMap::<Name<'a>, Dict<'a>>::default();

    context.save_state();

    while let Some(op) = ops.next() {
        match op {
            TypedInstruction::SaveState(_) => context.save_state(),
            TypedInstruction::StrokeColorDeviceRgb(s) => {
                context.stroke_paint_mut().color_space = ColorSpace::device_rgb();
                context.stroke_paint_mut().color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32()];
                context.stroke_paint_mut().pattern = None;
            }
            TypedInstruction::StrokeColorDeviceGray(s) => {
                context.stroke_paint_mut().color_space = ColorSpace::device_gray();
                context.stroke_paint_mut().color = smallvec![s.0.as_f32()];
                context.stroke_paint_mut().pattern = None;
            }
            TypedInstruction::StrokeColorCmyk(s) => {
                context.stroke_paint_mut().color_space = ColorSpace::device_cmyk();
                context.stroke_paint_mut().color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32(), s.3.as_f32()];
                context.stroke_paint_mut().pattern = None;
            }
            TypedInstruction::LineWidth(w) => {
                context.stroke_props_mut().line_width = w.0.as_f32();
            }
            TypedInstruction::LineCap(c) => {
                context.stroke_props_mut().line_cap = convert_line_cap(c);
            }
            TypedInstruction::LineJoin(j) => {
                context.stroke_props_mut().line_join = convert_line_join(j);
            }
            TypedInstruction::MiterLimit(l) => {
                context.stroke_props_mut().miter_limit = l.0.as_f32();
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

                let p = Point::new(r.0.as_f64(), r.1.as_f64());
                *(context.last_point_mut()) = p;
                *(context.sub_path_start_mut()) = p;
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
                context.fill_paint_mut().color_space = ColorSpace::device_gray();
                context.fill_paint_mut().color = smallvec![s.0.as_f32()];
                context.fill_paint_mut().pattern = None;
            }
            TypedInstruction::NonStrokeColorDeviceRgb(s) => {
                context.fill_paint_mut().color_space = ColorSpace::device_rgb();
                context.fill_paint_mut().color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32()];
                context.fill_paint_mut().pattern = None;
            }
            TypedInstruction::NonStrokeColorCmyk(s) => {
                context.fill_paint_mut().color_space = ColorSpace::device_cmyk();
                context.fill_paint_mut().color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32(), s.3.as_f32()];
                context.fill_paint_mut().pattern = None;
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

                    context.path_mut().curve_to(p1, p2, p3);
                }
            }
            TypedInstruction::CubicStartTo(c) => {
                if !context.path().elements().is_empty() {
                    let p1 = *context.last_point();
                    let p2 = Point::new(c.0.as_f64(), c.1.as_f64());
                    let p3 = Point::new(c.2.as_f64(), c.3.as_f64());

                    *(context.last_point_mut()) = p3;

                    context.path_mut().curve_to(p1, p2, p3);
                }
            }
            TypedInstruction::CubicEndTo(c) => {
                if !context.path().elements().is_empty() {
                    let p2 = Point::new(c.0.as_f64(), c.1.as_f64());
                    let p3 = Point::new(c.2.as_f64(), c.3.as_f64());

                    *(context.last_point_mut()) = p3;

                    context.path_mut().curve_to(p2, p3, p3);
                }
            }
            TypedInstruction::ClosePath(_) => {
                close_path(context);
            }
            TypedInstruction::SetGraphicsState(gs) => {
                if let Some(gs) = resources.get_ext_g_state(gs.0) {
                    handle_gs(&gs, context, resources);
                }
            }
            TypedInstruction::StrokePath(_) => {
                stroke_path(context, device);
            }
            TypedInstruction::EndPath(_) => end_path(context, device),
            TypedInstruction::NonStrokeColor(c) => {
                let paint = context.fill_paint_mut();
                paint.color = c.0.into_iter().map(|n| n.as_f32()).collect();
                paint.pattern = None;
            }
            TypedInstruction::StrokeColor(c) => {
                let paint = context.stroke_paint_mut();
                paint.color = c.0.into_iter().map(|n| n.as_f32()).collect();
                paint.pattern = None;
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
                let cs = if let Some(named) = ColorSpace::new_from_name(c.0) {
                    named
                } else {
                    context
                        .get_color_space(resources, c.0)
                        .unwrap_or(ColorSpace::device_gray())
                };

                if !cs.is_pattern() {
                    context.stroke_paint_mut().pattern = None;
                }
                context.stroke_paint_mut().color = cs.initial_color();
                context.stroke_paint_mut().color_space = cs;
            }
            TypedInstruction::ColorSpaceNonStroke(c) => {
                let cs = if let Some(named) = ColorSpace::new_from_name(c.0) {
                    named
                } else {
                    context
                        .get_color_space(resources, c.0)
                        .unwrap_or(ColorSpace::device_gray())
                };

                if !cs.is_pattern() {
                    context.fill_paint_mut().pattern = None;
                }
                context.fill_paint_mut().color = cs.initial_color();
                context.fill_paint_mut().color_space = cs;
            }
            TypedInstruction::DashPattern(p) => {
                context.stroke_props_mut().dash_offset = p.1.as_f32();
                // kurbo apparently cannot properly deal with offsets that are exactly 0.
                context.stroke_props_mut().dash_array =
                    p.0.iter::<f32>()
                        .map(|n| if n == 0.0 { 0.01 } else { n })
                        .collect();
            }
            TypedInstruction::RenderingIntent(_) => {
                // Ignore for now.
            }
            TypedInstruction::NonStrokeColorNamed(n) => {
                context.fill_paint_mut().color = n.0.into_iter().map(|n| n.as_f32()).collect();
                context.fill_paint_mut().pattern = n.1.and_then(|name| {
                    resources
                        .get_pattern(name)
                        .and_then(|d| Pattern::new(d, context))
                });
            }
            TypedInstruction::StrokeColorNamed(n) => {
                context.stroke_paint_mut().color = n.0.into_iter().map(|n| n.as_f32()).collect();
                context.stroke_paint_mut().pattern = n.1.and_then(|name| {
                    resources
                        .get_pattern(name)
                        .and_then(|d| Pattern::new(d, context))
                });
            }
            TypedInstruction::BeginMarkedContentWithProperties(bdc) => {
                // Properties can be either:
                // 1. A Name that references an entry in the Resources/Properties dictionary
                // 2. An inline property-list dictionary

                let properties = bdc
                    .1
                    .clone()
                    .into_name()
                    .and_then(|name| resources.properties.get::<Dict<'_>>(name))
                    .or_else(|| dict_or_stream(bdc.1).map(|(props, _)| props.clone()));

                let oc = bdc
                    .1
                    .clone()
                    .into_name()
                    .and_then(|name| {
                        let r = resources.properties.get_ref(name.as_ref())?;
                        let d = resources
                            .properties
                            .get::<Dict<'_>>(name)
                            .unwrap_or_default();
                        Some((d, r))
                    })
                    .or_else(|| {
                        let (props, _) = dict_or_stream(bdc.1)?;
                        let r = props.get_ref(OC)?;
                        let d = props.get::<Dict<'_>>(OC).unwrap_or_default();
                        Some((d, r))
                    });

                if let Some((dict, oc_ref)) = oc {
                    context.ocg_state.begin_ocg(&dict, oc_ref.into());
                } else {
                    context.ocg_state.begin_marked_content();
                }

                device.begin_marked_content(bdc.0, properties.as_ref());
            }
            TypedInstruction::MarkedContentPointWithProperties(_) => {}
            TypedInstruction::EndMarkedContent(_) => {
                context.ocg_state.end_marked_content();
                device.end_marked_content();
            }
            TypedInstruction::MarkedContentPoint(_) => {}
            TypedInstruction::BeginMarkedContent(bmc) => {
                context.ocg_state.begin_marked_content();
                device.begin_marked_content(bmc.0, None);
            }
            TypedInstruction::BeginText(_) => {
                context.text_state_mut().text_matrix = Affine::IDENTITY;
                context.text_state_mut().text_line_matrix = Affine::IDENTITY;
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
                context.text_state_mut().text_line_matrix = m;
                context.text_state_mut().text_matrix = m;
            }
            TypedInstruction::EndText(_) => {
                let has_outline = context.text_state().clip_paths.segments().next().is_some();

                if has_outline {
                    let clip_path = context.ctm() * context.text_state().clip_paths.clone();

                    context.push_clip_path(clip_path, FillRule::NonZero, device);
                }

                context.text_state_mut().clip_paths.truncate(0);
            }
            TypedInstruction::TextFont(t) => {
                let name = t.0;

                // In case we are unable to resolve the font, two scenarios:
                // 1) If the font doesn't exist in the first place in the resource dictionary,
                // assume Helvetica (this seems to be what other PDF viewers do).
                // 2) In case it's `None` because we were unable to resolve the font
                // (for whatever reason), leave it as `None`. Better showing no
                // text at all than garbage text.
                let font = if let Some(font_dict) = font_dict_cache.get(name).cloned() {
                    context.resolve_font(&font_dict)
                } else if let Some(font_dict) = resources.get_font(name) {
                    font_dict_cache.insert(name.clone(), font_dict.clone());
                    context.resolve_font(&font_dict)
                } else {
                    Font::new_standard(StandardFont::Helvetica, &context.settings.font_resolver)
                        .map(TextStateFont::Fallback)
                };

                context.text_state_mut().font_size = t.1.as_f32();
                context.text_state_mut().font = font;
            }
            TypedInstruction::ShowText(s) => {
                if context.text_state().font.is_none() {
                    // Even if no explicit font was set, we try to assume Helvetica. Acrobat
                    // seems to do the same.
                    context.text_state_mut().font = Font::new_standard(
                        StandardFont::Helvetica,
                        &context.settings.font_resolver,
                    )
                    .map(TextStateFont::Fallback);
                }

                text::show_text_string(context, device, resources, s.0);
            }
            TypedInstruction::ShowTexts(s) => {
                if context.text_state().font.is_none() {
                    // Even if no explicit font was set, we try to assume Helvetica. Acrobat
                    // seems to do the same.
                    context.text_state_mut().font = Font::new_standard(
                        StandardFont::Helvetica,
                        &context.settings.font_resolver,
                    )
                    .map(TextStateFont::Fallback);
                }

                text::begin_glyph_run(context);
                for obj in s.0.iter::<Object<'_>>() {
                    match obj {
                        Object::Number(num) => {
                            text::apply_glyph_run_adjustment(context, num.as_f32());
                        }
                        Object::String(text) => {
                            text::append_text_string(context, resources, &text);
                        }
                        _ => {}
                    }
                }
                text::show_glyph_run(context, device);
            }
            TypedInstruction::HorizontalScaling(h) => {
                context.text_state_mut().horizontal_scaling = h.0.as_f32();
            }
            TypedInstruction::TextLeading(tl) => {
                context.text_state_mut().leading = tl.0.as_f32();
            }
            TypedInstruction::CharacterSpacing(c) => {
                context.text_state_mut().char_space = c.0.as_f32();
            }
            TypedInstruction::WordSpacing(w) => {
                context.text_state_mut().word_space = w.0.as_f32();
            }
            TypedInstruction::NextLine(n) => {
                let (tx, ty) = (n.0.as_f64(), n.1.as_f64());
                text::next_line(context, tx, ty);
            }
            TypedInstruction::NextLineUsingLeading(_) => {
                text::next_line(context, 0.0, -context.text_state().leading as f64);
            }
            TypedInstruction::NextLineAndShowText(n) => {
                text::next_line(context, 0.0, -context.text_state().leading as f64);
                text::show_text_string(context, device, resources, n.0);
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
                    _ => TextRenderingMode::Fill,
                };

                context.text_state_mut().render_mode = mode;
            }
            TypedInstruction::NextLineAndSetLeading(n) => {
                let (tx, ty) = (n.0.as_f64(), n.1.as_f64());
                context.text_state_mut().leading = -ty as f32;
                text::next_line(context, tx, ty);
            }
            TypedInstruction::ShapeGlyph(_) => {}
            TypedInstruction::XObject(x) => {
                let cache = context.interpreter_cache.object_cache.clone();
                let warning_sink = context.settings.warning_sink.clone();
                let transfer_function = context.compositing().transfer_function.clone();
                if let Some(x_object) = resources.get_x_object(x.0).and_then(|s| {
                    XObject::new(
                        &s,
                        |name| context.get_color_space(resources, name),
                        &warning_sink,
                        &cache,
                        transfer_function.clone(),
                    )
                }) {
                    x_object.draw(resources, context, device);
                }
            }
            TypedInstruction::InlineImage(i) => {
                let warning_sink = context.settings.warning_sink.clone();
                let transfer_function = context.compositing().transfer_function.clone();
                let cache = context.interpreter_cache.object_cache.clone();
                if let Some(x_object) = ImageXObject::new(
                    i.0,
                    |name| context.get_color_space(resources, name),
                    &warning_sink,
                    &cache,
                    transfer_function,
                ) {
                    x_object.draw(context, device);
                }
            }
            TypedInstruction::TextRise(t) => {
                context.text_state_mut().rise = t.0.as_f32();
            }
            TypedInstruction::Shading(s) => {
                if !context.ocg_state.is_visible() {
                    continue;
                }

                let transfer_function = context.compositing().transfer_function.clone();

                if let Some(mut shading) = resources.get_shading(s.0).and_then(|o| {
                    let (dict, stream) = dict_or_stream(&o)?;
                    Shading::new(dict, stream, &context.interpreter_cache.object_cache)
                }) {
                    // The background only applies to shading patterns, not to `sh`.
                    shading.background = None;

                    let mut bbox = context.bbox();
                    if let Some(bounds) = shading.mesh_bounds {
                        // Snapped outwards to whole pixels, so that the sampled texture stays
                        // aligned with the device pixels.
                        let bounds = context.ctm().transform_rect_bbox(bounds);
                        bbox = bbox.intersect(bounds.expand().inflate(1.0, 1.0));
                    }

                    context.save_state();
                    context.push_root_transform();
                    let paint = context.fill_paint_mut();
                    paint.pattern = Some(Pattern::Shading(ShadingPattern {
                        shading: Arc::new(shading),
                        matrix: Affine::IDENTITY,
                        opacity: 1.0,
                        transfer_function: transfer_function.clone(),
                    }));
                    paint.color_space = ColorSpace::pattern();

                    let bbox = bbox.to_path(0.1);
                    let inverted_bbox = context.ctm().inverse() * bbox;
                    fill_path_impl(context, device, FillRule::NonZero, Some(&inverted_bbox));

                    context.pop_root_transform();
                    context.restore_state(device);
                }
            }
            TypedInstruction::BeginCompatibility(_) => {}
            TypedInstruction::EndCompatibility(_) => {}
            TypedInstruction::ColorGlyph(_) => {}
            TypedInstruction::ShowTextWithParameters(t) => {
                context.text_state_mut().word_space = t.0.as_f32();
                context.text_state_mut().char_space = t.1.as_f32();
                text::next_line(context, 0.0, -context.text_state().leading as f64);
                text::show_text_string(context, device, resources, t.2);
            }
            _ => {}
        }
    }

    // A path object, including a pending `W` or `W*`, can't continue beyond the content
    // stream it starts in.
    context.path_mut().truncate(0);
    *(context.clip_mut()) = None;

    while context.num_states() > num_states {
        context.restore_state(device);
    }
}
