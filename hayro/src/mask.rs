use crate::{GlobalState, Renderer, convert_blend_mode, derive_settings};
use hayro_interpret::{BlendMode, CacheKey, MaskType, SoftMask};
use kurbo::Rect;
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
    let mut renderer = Renderer::new(width, height, derive_settings(&settings), global);

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

    mask.interpret(&mut renderer);

    if apply_bg {
        renderer.ctx.pop_layer();
    }

    let mut pix = Pixmap::new(width, height);
    renderer.ctx.flush();
    let mut resources = vello_cpu::Resources::default();
    renderer.ctx.render(&mut pix, &mut resources);

    let mut rendered_mask = match mask.mask_type() {
        MaskType::Luminosity => Mask::new_luminance(&pix),
        MaskType::Alpha => Mask::new_alpha(&pix),
    };

    if let Some(transfer_function) = mask.transfer_function() {
        // TODO: Allow deconstructing a mask in Vello CPU.
        let mut map = Vec::with_capacity(
            usize::from(rendered_mask.width()) * usize::from(rendered_mask.height()),
        );

        for y in 0..rendered_mask.height() {
            for x in 0..rendered_mask.width() {
                map.push(rendered_mask.sample(x, y));
            }
        }

        transfer_function.apply_to(&mut map);
        rendered_mask = Mask::from_parts(map, rendered_mask.width(), rendered_mask.height());
    }

    rendered_mask
}
