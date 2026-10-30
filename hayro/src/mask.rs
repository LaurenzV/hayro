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
    let mut ctx = vello_cpu::RenderContext::new_with(width, height, derive_settings(&settings));
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

    mask.interpret(&mut renderer);

    if apply_bg {
        renderer.ctx.pop_layer();
    }

    let mut pix = Pixmap::new(width, height);
    renderer.ctx.flush();
    let mut resources = vello_cpu::Resources::default();
    renderer.ctx.render(&mut pix, &mut resources);

    let mut values = match mask.mask_type() {
        MaskType::Luminosity => luminance(&pix),
        MaskType::Alpha => pix.data().iter().map(|pixel| pixel.a).collect(),
    };

    if let Some(transfer_function) = mask.transfer_function() {
        transfer_function.apply_to(&mut values);
    }

    Mask::from_parts(values, width, height)
}

/// The luminance of each pixel of a premultiplied pixmap, as `Mask::new_luminance` computes it
/// (its weighted channels added in the same order, so the result is the same), with each
/// channel's weighted values looked up rather than divided and multiplied for every pixel.
pub(crate) fn luminance(pixmap: &Pixmap) -> Vec<u8> {
    let [r, g, b] = [0.2126_f32, 0.7152, 0.0722]
        .map(|weight| std::array::from_fn::<f32, 256, _>(|v| (v as f32 / 255.0) * weight));

    pixmap
        .data()
        .iter()
        .map(|pixel| {
            let luma = r[usize::from(pixel.r)] + g[usize::from(pixel.g)] + b[usize::from(pixel.b)];
            (luma * 255.0 + 0.5) as u8
        })
        .collect()
}
