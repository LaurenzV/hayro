use crate::encode::{Buffer, x_y_advances};
use crate::mask::Mask;
use crate::paint::{Image, PaintType};
use crate::renderer::Renderer;
use hayro_interpret::Context;
use hayro_interpret::Device;
use hayro_interpret::color::AlphaColor;
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::object::ObjectIdentifier;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::util::FloatExt;
use hayro_interpret::{ClipPath, LumaData};
use hayro_interpret::{
    FillProps, FillRule, MaskType, Paint, RgbData, SoftMask, StrokeProps, interpret,
};
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageBuffer, ImageEncoder, RgbImage};
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use std::collections::HashMap;
use std::io::Cursor;
use std::ops::RangeInclusive;
use std::sync::Arc;

use crate::ctx::RenderContext;
pub use hayro_interpret::font::{FontData, FontQuery, StandardFont};
pub use hayro_interpret::{InterpreterSettings, Pdf};
pub use pixmap::Pixmap;

mod coarse;
mod ctx;
mod encode;
mod fine;
mod flatten;
mod mask;
mod paint;
mod pixmap;
mod renderer;
mod strip;
mod tile;

/// Settings to apply during rendering.
pub struct RenderSettings {
    /// How much the contents should be scaled into the x direction.
    pub x_scale: f32,
    /// How much the contents should be scaled into the y direction.
    pub y_scale: f32,
    /// The width of the viewport. If this is set to `None`, the width will be chosen
    /// automatically based on the scale factor and the dimensions of the PDF.
    pub width: Option<u16>,
    /// The height of the viewport. If this is set to `None`, the height will be chosen
    /// automatically based on the scale factor and the dimensions of the PDF.
    pub height: Option<u16>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            x_scale: 1.0,
            y_scale: 1.0,
            width: None,
            height: None,
        }
    }
}

pub fn render(
    page: &Page,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
) -> Pixmap {
    let (x_scale, y_scale) = (render_settings.x_scale, render_settings.y_scale);
    let (width, height) = page.render_dimensions();
    let (scaled_width, scaled_height) = ((width * x_scale) as f64, (height * y_scale) as f64);
    let initial_transform =
        Affine::scale_non_uniform(x_scale as f64, y_scale as f64) * page.initial_transform(true);

    let (pix_width, pix_height) = (
        render_settings.width.unwrap_or(scaled_width.floor() as u16),
        render_settings
            .height
            .unwrap_or(scaled_height.floor() as u16),
    );
    let mut state = Context::new(
        initial_transform,
        Rect::new(0.0, 0.0, pix_width as f64, pix_height as f64),
        page.xref(),
        interpreter_settings.clone(),
    );
    let mut device = Renderer {
        ctx: RenderContext::new(pix_width, pix_height),
        inside_pattern: false,
        soft_mask_cache: Default::default(),
        cur_mask: None,
    };

    device.ctx.fill_rect(
        &Rect::new(0.0, 0.0, pix_width as f64, pix_height as f64),
        AlphaColor::WHITE.into(),
        Affine::IDENTITY,
        None,
    );
    device.push_clip_path(&ClipPath {
        path: initial_transform * page.intersected_crop_box().to_path(0.1),
        fill: FillRule::NonZero,
    });

    device.set_transform(initial_transform);

    interpret(
        page.typed_operations(),
        page.resources(),
        &mut state,
        &mut device,
    );

    device.pop_clip_path();

    let mut pixmap = Pixmap::new(pix_width, pix_height);
    device.ctx.render_to_pixmap(&mut pixmap);

    pixmap
}

pub fn render_png(
    pdf: &Pdf,
    scale: f32,
    settings: InterpreterSettings,
    range: Option<RangeInclusive<usize>>,
) -> Option<Vec<Vec<u8>>> {
    let rendered = pdf
        .pages()
        .iter()
        .enumerate()
        .flat_map(|(idx, page)| {
            if range.clone().is_some_and(|range| !range.contains(&idx)) {
                return None;
            }

            let pixmap = render(
                page,
                &settings,
                &RenderSettings {
                    x_scale: scale,
                    y_scale: scale,
                    ..Default::default()
                },
            );

            let mut png_data = Vec::new();
            let cursor = Cursor::new(&mut png_data);
            let encoder = PngEncoder::new(cursor);
            encoder
                .write_image(
                    pixmap.data_as_u8_slice(),
                    pixmap.width() as u32,
                    pixmap.height() as u32,
                    ExtendedColorType::Rgba8,
                )
                .expect("Failed to encode image");

            Some(png_data)
        })
        .collect();

    Some(rendered)
}
