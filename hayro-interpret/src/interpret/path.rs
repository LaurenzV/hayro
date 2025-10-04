use crate::color::{Color, ColorSpace};
use crate::context::Context;
use crate::device::Device;
use crate::util::Float32Ext;
use crate::{FillRule, Paint, PathDrawMode, StrokeProps};
use kurbo::{BezPath, Cap, Join, PathEl, Shape};
use smallvec::smallvec;

pub(crate) fn fill_path<'a>(
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
    fill_rule: FillRule,
) {
    fill_path_impl(context, device, fill_rule, None);

    context.path_mut().truncate(0);
}

pub(crate) fn stroke_path<'a>(context: &mut Context<'a>, device: &mut impl Device<'a>) {
    stroke_path_impl(context, device, None);

    context.path_mut().truncate(0);
}

pub(crate) fn fill_stroke_path<'a>(
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
    fill_rule: FillRule,
) {
    fill_path_impl(context, device, fill_rule, None);
    stroke_path_impl(context, device, None);

    context.path_mut().truncate(0);
}

pub(crate) fn close_path(context: &mut Context<'_>) {
    // This is necessary to prevent artifacts (see for example issue 157),
    // but it does cause some weird lines (maybe conflation artifacts) in
    // pdftc_900k_0907.
    if context.path().elements().last() != Some(&PathEl::ClosePath) && !context.path().is_empty() {
        context.path_mut().close_path();

        *(context.last_point_mut()) = *context.sub_path_start();
    }
}

pub(crate) fn fill_path_impl<'a>(
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
    fill_rule: FillRule,
    path: Option<&BezPath>,
) {
    if !context.ocg_state.is_visible() {
        return;
    }

    let base_transform = context.get().ctm;

    let paint = get_paint(context, false);
    device.set_soft_mask(context.get().graphics_state.soft_mask.clone());
    device.set_blend_mode(context.get().graphics_state.blend_mode);

    let mut draw = |path: &BezPath| {
        // pdf.js issue 4260: Replace zero-sized paths with a small stroke instead.
        let bbox = path.bounding_box();

        match (
            (bbox.width() as f32).is_nearly_zero(),
            (bbox.height() as f32).is_nearly_zero(),
        ) {
            (false, false) => {
                device.draw_path(path, base_transform, &paint, &PathDrawMode::Fill(fill_rule))
            }
            _ => {
                let mut path = BezPath::new();
                path.move_to((bbox.x0, bbox.y0));
                path.line_to((bbox.x1, bbox.y1));

                let stroke_props = StrokeProps {
                    // TODO: Make dependent on transform?
                    line_width: 0.001,
                    line_join: Join::Bevel,
                    line_cap: Cap::Butt,
                    ..Default::default()
                };

                device.draw_path(
                    &path,
                    base_transform,
                    &paint,
                    &PathDrawMode::Stroke(stroke_props),
                );
            }
        };
    };

    match path {
        None => draw(context.path()),
        Some(path) => draw(path),
    };
}

pub(crate) fn stroke_path_impl<'a>(
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
    path: Option<&BezPath>,
) {
    if !context.ocg_state.is_visible() {
        return;
    }

    let base_transform = context.get().ctm;

    let stroke_props = context.stroke_props();
    device.set_soft_mask(context.get().graphics_state.soft_mask.clone());
    device.set_blend_mode(context.get().graphics_state.blend_mode);
    let paint = get_paint(context, true);

    match path {
        None => device.draw_path(
            context.path(),
            base_transform,
            &paint,
            &PathDrawMode::Stroke(stroke_props),
        ),
        Some(path) => device.draw_path(
            path,
            base_transform,
            &paint,
            &PathDrawMode::Stroke(stroke_props),
        ),
    };
}

pub(crate) fn get_paint<'a>(context: &Context<'a>, is_stroke: bool) -> Paint<'a> {
    let data = if is_stroke {
        context.get().stroke_data()
    } else {
        context.get().non_stroke_data()
    };

    if data.color_space.is_pattern() {
        if let Some(mut pattern) = data.pattern {
            pattern.pre_concat_transform(context.root_transform());

            Paint::Pattern(Box::new(pattern))
        } else {
            // Pattern was likely invalid, use transparent paint.
            Paint::Color(Color::new(ColorSpace::device_gray(), smallvec![0.0], 0.0))
        }
    } else {
        let color = Color::new(data.color_space, data.color, data.alpha);

        Paint::Color(color)
    }
}
