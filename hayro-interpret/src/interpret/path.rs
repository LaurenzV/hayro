use crate::color::Color;
use crate::context::Context;
use crate::device::Device;
use crate::{Paint, PathDrawMode};
use kurbo::{BezPath, PathEl};

pub(crate) fn fill_path<'a>(context: &mut Context<'a>, device: &mut impl Device<'a>) {
    fill_path_impl(context, device, None);

    context.path_mut().truncate(0);
}

pub(crate) fn stroke_path<'a>(context: &mut Context<'a>, device: &mut impl Device<'a>) {
    stroke_path_impl(context, device, None);

    context.path_mut().truncate(0);
}

pub(crate) fn fill_stroke_path<'a>(context: &mut Context<'a>, device: &mut impl Device<'a>) {
    fill_path_impl(context, device, None);
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
    path: Option<&BezPath>,
) {
    let base_transform = context.get().ctm;

    let paint = get_paint(context, false);
    let fill_rule = context.fill_rule();
    device.set_soft_mask(context.get().soft_mask.clone());

    match path {
        None => device.draw_path(
            context.path(),
            base_transform,
            &paint,
            &PathDrawMode::Fill(fill_rule),
        ),
        Some(path) => {
            device.draw_path(path, base_transform, &paint, &PathDrawMode::Fill(fill_rule))
        }
    };
}

pub(crate) fn stroke_path_impl<'a>(
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
    path: Option<&BezPath>,
) {
    let base_transform = context.get().ctm;

    let stroke_props = context.stroke_props();
    device.set_soft_mask(context.get().soft_mask.clone());
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

    if data.color_space.is_pattern()
        && let Some(mut pattern) = data.pattern
    {
        pattern.pre_concat_transform(context.root_transform());

        Paint::Pattern(Box::new(pattern))
    } else {
        let color = Color::new(data.color_space, data.color, data.alpha);

        Paint::Color(color)
    }
}
