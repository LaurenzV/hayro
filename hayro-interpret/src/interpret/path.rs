use crate::Paint;
use crate::PaintType;
use crate::color::Color;
use crate::context::Context;
use crate::device::Device;
use kurbo::{Affine, BezPath};

pub(crate) fn fill_path(context: &mut Context, device: &mut impl Device) {
    fill_path_impl(context, device, None);

    context.path_mut().truncate(0);
}

pub(crate) fn stroke_path(context: &mut Context, device: &mut impl Device) {
    stroke_path_impl(context, device, None);

    context.path_mut().truncate(0);
}

pub(crate) fn fill_stroke_path(context: &mut Context, device: &mut impl Device) {
    fill_path_impl(context, device, None);
    stroke_path_impl(context, device, None);

    context.path_mut().truncate(0);
}

pub(crate) fn fill_path_impl(
    context: &mut Context,
    device: &mut impl Device,
    path: Option<&BezPath>,
) {
    let base_transform = context.get().ctm;

    let paint = get_paint(context, false);
    let fill_rule = context.fill_rule();
    device.set_soft_mask(context.get().soft_mask.clone());

    match path {
        None => device.fill_path(context.path(), base_transform, &paint, fill_rule),
        Some(path) => device.fill_path(path, base_transform, &paint, fill_rule),
    };
}

pub(crate) fn stroke_path_impl(
    context: &mut Context,
    device: &mut impl Device,
    path: Option<&BezPath>,
) {
    let base_transform = context.get().ctm;

    let stroke_props = context.stroke_props();
    device.set_soft_mask(context.get().soft_mask.clone());
    let paint = get_paint(context, true);

    match path {
        None => device.stroke_path(context.path(), base_transform, &paint, &stroke_props),
        Some(path) => device.stroke_path(path, base_transform, &paint, &stroke_props),
    };
}

pub(crate) fn get_paint<'a>(context: &Context<'a>, is_stroke: bool) -> Paint<'a> {
    let data = if is_stroke {
        context.get().stroke_data()
    } else {
        context.get().non_stroke_data()
    };

    if data.color_space.is_pattern()
        && let Some(pattern) = data.pattern
    {
        Paint {
            paint_type: PaintType::Pattern(Box::new(pattern)),
            paint_transform: context.root_transform(),
        }
    } else {
        let color = Color::new(data.color_space, data.color, data.alpha);

        Paint {
            paint_type: PaintType::Color(color),
            paint_transform: Affine::IDENTITY,
        }
    }
}
