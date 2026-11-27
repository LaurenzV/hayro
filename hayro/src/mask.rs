use crate::{GlobalState, Renderer, convert_blend_mode, derive_settings};
use hayro_interpret::{BlendMode, CacheKey, MaskType, SoftMask};
use kurbo::{Affine, Rect};
use vello_cpu::color::palette::css::BLACK;
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::{Mask, Pixmap, RenderSettings};

impl Renderer<'_> {
    pub(super) fn apply_soft_mask(&mut self, mask: Option<&SoftMask<'_>>) {
        let settings = *self.ctx.render_settings();
        let global = self.global;
        let mask = mask.map(|m| {
            let width = self.ctx.width();
            let height = self.ctx.height();

            self.soft_mask_cache
                .entry(m.cache_key())
                .or_insert_with(|| draw_soft_mask(m, settings, width, height, global))
                .clone()
        });

        if let Some(mask) = mask {
            self.ctx.set_mask(mask);
        } else {
            self.ctx.reset_mask();
        }
    }

    pub(super) fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'_>>,
        blend_mode: BlendMode,
    ) {
        let settings = *self.ctx.render_settings();
        let global = self.global;
        self.ctx.push_layer(
            None,
            Some(convert_blend_mode(blend_mode)),
            Some(opacity),
            // TODO: Deduplicate
            mask.map(|m| {
                let width = self.ctx.width();
                let height = self.ctx.height();

                self.soft_mask_cache
                    .entry(m.cache_key())
                    .or_insert_with(|| draw_soft_mask(&m, settings, width, height, global))
                    .clone()
            }),
            None,
        );
    }

    pub(super) fn pop_transparency_group(&mut self) {
        self.ctx.pop_layer();
    }
}

fn draw_soft_mask(
    mask: &SoftMask<'_>,
    settings: RenderSettings,
    width: u16,
    height: u16,
    global: &GlobalState,
) -> Mask {
    let canvas = (0, 0, width, height);
    let region = group_region(mask.group_bbox(), width, height);

    if region == Some(canvas) {
        let rendered = draw_region(mask, settings, global, canvas, true);

        return match mask.transfer_function() {
            Some(transfer_function) => {
                let mut data = mask_data(&rendered);
                transfer_function.apply_to(&mut data);
                Mask::from_parts(data, width, height)
            }
            None => rendered,
        };
    }

    // Outside of its bounding box, the group paints nothing, so the mask only
    // has the value of its backdrop there. Instead of rendering the whole
    // canvas, we render the backdrop into a single pixel and the group into
    // the region it covers, and put the mask together from both.
    let mut outside = [draw_region(mask, settings, global, (0, 0, 1, 1), false).sample(0, 0)];
    apply_transfer_function(mask, &mut outside);
    let mut data = vec![outside[0]; usize::from(width) * usize::from(height)];

    if let Some(region @ (x, y, region_width, _)) = region {
        let mut region_data = mask_data(&draw_region(mask, settings, global, region, true));
        apply_transfer_function(mask, &mut region_data);

        let (x, y, width) = (usize::from(x), usize::from(y), usize::from(width));
        let region_width = usize::from(region_width);

        for (row, region_row) in region_data.chunks_exact(region_width).enumerate() {
            let start = (y + row) * width + x;
            data[start..start + region_width].copy_from_slice(region_row);
        }
    }

    Mask::from_parts(data, width, height)
}

/// Return the pixels `(x, y, width, height)` of the canvas that `bbox`
/// touches, or `None` if it doesn't touch any.
fn group_region(bbox: Rect, width: u16, height: u16) -> Option<(u16, u16, u16, u16)> {
    let canvas = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));

    if ![bbox.x0, bbox.y0, bbox.x1, bbox.y1]
        .iter()
        .all(|v| v.is_finite())
    {
        return Some((0, 0, width, height));
    }

    let region = bbox.expand().intersect(canvas);

    if region.width() <= 0.0 || region.height() <= 0.0 {
        return None;
    }

    // The region is an integer rectangle within the canvas.
    Some((
        region.x0 as u16,
        region.y0 as u16,
        region.width() as u16,
        region.height() as u16,
    ))
}

/// Draw the mask into the pixels `(x, y, width, height)` of the canvas,
/// without applying its transfer function.
///
/// Without `interpret_group`, only the backdrop is drawn.
fn draw_region(
    mask: &SoftMask<'_>,
    settings: RenderSettings,
    global: &GlobalState,
    (x, y, width, height): (u16, u16, u16, u16),
    interpret_group: bool,
) -> Mask {
    let mut ctx = vello_cpu::RenderContext::new_with(width, height, derive_settings(&settings));
    ctx.set_aliasing_threshold(global.aliasing_threshold);
    let mut renderer = Renderer::new(&mut ctx, global);

    let bg_color = mask.background_color().to_rgba();
    let apply_bg = bg_color.to_rgba8() != BLACK.to_rgba8().to_u8_array();

    if apply_bg {
        renderer
            .ctx
            .set_paint(AlphaColor::<Srgb>::new(bg_color.components()));
        renderer
            .ctx
            .fill_rect(&Rect::new(0.0, 0.0, width as f64, height as f64));
        renderer.ctx.push_layer(None, None, None, None, None);
    }

    if interpret_group {
        let offset = Affine::translate((-f64::from(x), -f64::from(y)));
        mask.interpret_with_transform(&mut renderer, offset);
    }

    if apply_bg {
        renderer.ctx.pop_layer();
    }

    let mut pix = Pixmap::new(width, height);
    renderer.ctx.flush();
    let mut resources = vello_cpu::Resources::default();
    renderer.ctx.render(&mut pix, &mut resources);

    match mask.mask_type() {
        MaskType::Luminosity => Mask::new_luminance(&pix),
        MaskType::Alpha => Mask::new_alpha(&pix),
    }
}

// TODO: Allow deconstructing a mask in Vello CPU.
fn mask_data(mask: &Mask) -> Vec<u8> {
    let mut data = Vec::with_capacity(usize::from(mask.width()) * usize::from(mask.height()));

    for y in 0..mask.height() {
        for x in 0..mask.width() {
            data.push(mask.sample(x, y));
        }
    }

    data
}

fn apply_transfer_function(mask: &SoftMask<'_>, data: &mut [u8]) {
    if let Some(transfer_function) = mask.transfer_function() {
        transfer_function.apply_to(data);
    }
}
