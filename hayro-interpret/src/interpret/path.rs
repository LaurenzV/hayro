use crate::context::{Context, path_as_rect};
use crate::device::Device;
use crate::util::{BezPathExt, Float32Ext};
use crate::{DrawMode, FillRule, StrokeProps};
use kurbo::{BezPath, Cap, Join, PathEl};

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

    let props = context.draw_props(false);

    let mut draw = |path: &BezPath| {
        // pdf.js issue 4260: Replace zero-sized paths with a small stroke instead.
        let bbox = path.fast_bounding_box();

        match (
            (bbox.width() as f32).is_nearly_zero(),
            (bbox.height() as f32).is_nearly_zero(),
        ) {
            (false, false) => {
                let draw_mode = DrawMode::Fill(fill_rule);
                if let Some(rect) = path_as_rect(path) {
                    device.draw_rect(&rect, props.clone(), &draw_mode);
                } else {
                    device.draw_path(path, props.clone(), &draw_mode);
                }
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

                device.draw_path(&path, props.clone(), &DrawMode::Stroke(stroke_props));
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

    let stroke_props = context.stroke_props();
    let props = context.draw_props(true);

    let path = path.unwrap_or(context.path());
    let draw_mode = DrawMode::Stroke(stroke_props);

    if let Some(rect) = path_as_rect(path) {
        device.draw_rect(&rect, props, &draw_mode);
    } else {
        device.draw_path(path, props, &draw_mode);
    }
}
