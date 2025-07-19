use crate::encode::{Buffer, x_y_advances};
use crate::mask::Mask;
use crate::paint::{Image, PaintType};
use crate::pixmap::Pixmap;
use crate::render::RenderContext;
use hayro_interpret::Context;
use hayro_interpret::Device;
pub use hayro_interpret::InterpreterSettings;
use hayro_interpret::color::AlphaColor;
pub use hayro_interpret::font::FontData;
pub use hayro_interpret::font::FontQuery;
use hayro_interpret::font::Glyph;
pub use hayro_interpret::font::StandardFont;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::util::FloatExt;
use hayro_interpret::{ClipPath, LumaData};
use hayro_interpret::{
    FillProps, FillRule, MaskType, Paint, RgbData, SoftMask, StrokeProps, interpret,
};
pub use hayro_syntax::Pdf;
use hayro_syntax::object::ObjectIdentifier;
use hayro_syntax::page::{A4, Page, Rotation};
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageBuffer, ImageEncoder, RgbImage};
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use std::collections::HashMap;
use std::io::Cursor;
use std::ops::RangeInclusive;
use std::sync::Arc;

mod coarse;
mod encode;
mod fine;
mod flatten;
mod mask;
mod paint;
mod pixmap;
pub mod render;
mod strip;
mod tile;

struct Renderer {
    ctx: RenderContext,
    inside_pattern: bool,
    soft_mask_cache: HashMap<ObjectIdentifier, Mask>,
    cur_mask: Option<Mask>,
}

impl Renderer {
    fn draw_image(&mut self, rgb_data: RgbData, alpha_data: Option<LumaData>, is_stencil: bool) {
        let mut cur_transform = self.ctx.transform;

        let (x_scale, y_scale) = {
            let (x, y) = x_y_advances(&cur_transform);
            (x.length() as f32, y.length() as f32)
        };
        let mut rgb_width = rgb_data.width;
        let mut rgb_height = rgb_data.height;

        let interpolate = rgb_data.interpolate;

        let rgb_data = if x_scale >= 1.0 && y_scale >= 1.0 {
            rgb_data.data
        } else {
            // Resize the image, either doing down- or upsampling.
            let new_width = (rgb_width as f32 * x_scale).ceil().max(1.0) as u32;
            let new_height = (rgb_height as f32 * y_scale).ceil().max(1.0) as u32;

            let image = DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(rgb_width, rgb_height, rgb_data.data.clone()).unwrap(),
            );
            let resized = image.resize_exact(new_width, new_height, FilterType::CatmullRom);

            let new_width = resized.width();
            let new_height = resized.height();
            let t_scale_x = rgb_width as f32 / new_width as f32;
            let t_scale_y = rgb_height as f32 / new_height as f32;

            cur_transform *= Affine::scale_non_uniform(t_scale_x as f64, t_scale_y as f64);
            self.ctx.set_transform(cur_transform);

            rgb_width = new_width;
            rgb_height = new_height;

            resized.to_rgb8().into_raw()
        };

        let alpha_data = if let Some(alpha_data) = alpha_data {
            if alpha_data.width != rgb_width || alpha_data.height != rgb_height {
                let image = DynamicImage::ImageLuma8(
                    ImageBuffer::from_raw(
                        alpha_data.width,
                        alpha_data.height,
                        alpha_data.data.clone(),
                    )
                    .unwrap(),
                );
                let resized = image.resize_exact(rgb_width, rgb_height, FilterType::CatmullRom);
                resized.to_luma8().into_raw()
            } else {
                alpha_data.data
            }
        } else {
            vec![255; rgb_width as usize * rgb_height as usize]
        };

        let rgba_data = rgb_data
            .chunks_exact(3)
            .zip(alpha_data)
            .flat_map(|(rgb, a)| [rgb[0], rgb[1], rgb[2], a])
            .collect::<Vec<_>>();

        let mut buffer = Buffer::<4>::new_u8(rgba_data, rgb_width, rgb_height);
        buffer.premultiply();

        let image = Image {
            buffer: Arc::new(buffer),
            interpolate,
            is_stencil,
            is_pattern: false,
        };

        self.ctx.fill_rect(
            &Rect::new(0.0, 0.0, rgb_width as f64, rgb_height as f64),
            image.into(),
            self.ctx.transform,
            None,
        );
    }

    fn convert_paint(
        &mut self,
        paint: &hayro_interpret::Paint,
        is_stroke: bool,
    ) -> (PaintType, Affine) {
        match paint.paint_type.clone() {
            hayro_interpret::PaintType::Color(c) => (c.to_rgba().into(), Affine::IDENTITY),
            hayro_interpret::PaintType::Pattern(p) => {
                match *p {
                    Pattern::Shading(s) => (s.into(), paint.paint_transform),
                    Pattern::Tiling(t) => {
                        const MAX_PIXMAP_SIZE: f32 = 3000.0;
                        // TODO: Raise this limit and perform downsampling if reached
                        // (see pdftc_100k_0138.pdf).
                        const MIN_PIXMAP_SIZE: f32 = 1.0;

                        let bbox = t.bbox;
                        let max_x_scale = MAX_PIXMAP_SIZE / bbox.width() as f32;
                        let min_x_scale = MIN_PIXMAP_SIZE / bbox.width() as f32;
                        let max_y_scale = MAX_PIXMAP_SIZE / bbox.height() as f32;
                        let min_y_scale = MIN_PIXMAP_SIZE / bbox.height() as f32;

                        let (mut xs, mut ys) = {
                            let (x, y) = x_y_advances(&(paint.paint_transform * t.matrix));
                            (x.length() as f32, y.length() as f32)
                        };
                        xs = xs.max(min_x_scale).min(max_x_scale);
                        ys = ys.max(min_y_scale).min(max_y_scale);

                        let mut x_step = xs * t.x_step;
                        let mut y_step = ys * t.y_step;

                        let scaled_width = bbox.width() as f32 * xs;
                        let scaled_height = bbox.height() as f32 * ys;
                        let pix_width = x_step.abs().round() as u16;
                        let pix_height = y_step.abs().round() as u16;

                        let mut renderer = Renderer {
                            ctx: RenderContext::new(pix_width, pix_height),
                            cur_mask: None,
                            inside_pattern: true,
                            soft_mask_cache: Default::default(),
                        };
                        let mut initial_transform =
                            Affine::new([xs as f64, 0.0, 0.0, ys as f64, -bbox.x0, -bbox.y0]);
                        t.interpret(&mut renderer, initial_transform, is_stroke);
                        let mut pix = Pixmap::new(pix_width, pix_height);
                        renderer.ctx.render_to_pixmap(&mut pix);

                        // TODO: Add tests
                        if x_step < 0.0 {
                            initial_transform *=
                                Affine::new([-1.0, 0.0, 0.0, 1.0, scaled_width as f64, 0.0]);
                            x_step = x_step.abs();
                        }

                        if y_step < 0.0 {
                            initial_transform *=
                                Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, scaled_height as f64]);
                            y_step = y_step.abs();
                        }

                        let buffer = Buffer::new_u8(
                            pix.data_as_u8_slice().to_vec(),
                            pix_width as u32,
                            pix_height as u32,
                        );

                        let final_transform =
                            paint.paint_transform * t.matrix * initial_transform.inverse();

                        let image = PaintType::Image(Image {
                            buffer: Arc::new(buffer),
                            interpolate: true,
                            is_stencil: false,
                            is_pattern: true,
                        });

                        (image, final_transform)
                    }
                }
            }
        }
    }
}

impl Device for Renderer {
    fn set_transform(&mut self, affine: Affine) {
        self.ctx.set_transform(affine);
    }

    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps) {
        // Best-effort attempt to ensure a line width of at least 1.
        let min_factor = min_factor(&self.ctx.transform);
        let mut line_width = stroke_props.line_width.max(0.01);
        let transformed_width = line_width * min_factor;

        // Only enforce line width if not inside of pattern.
        if transformed_width < 1.0 && !self.inside_pattern {
            line_width /= transformed_width;
        }

        let stroke = kurbo::Stroke {
            width: line_width as f64,
            join: stroke_props.line_join,
            miter_limit: stroke_props.miter_limit as f64,
            start_cap: stroke_props.line_cap,
            end_cap: stroke_props.line_cap,
            dash_pattern: stroke_props.dash_array.iter().map(|n| *n as f64).collect(),
            dash_offset: stroke_props.dash_offset as f64,
        };

        self.ctx.set_stroke(stroke);
    }

    fn stroke_path(&mut self, path: &BezPath, paint: &Paint) {
        let (paint_type, paint_transform) = self.convert_paint(paint, true);
        self.ctx
            .stroke_path(path, paint_type, paint_transform, self.cur_mask.clone());
    }

    fn set_fill_properties(&mut self, fill_props: &FillProps) {
        self.ctx.set_fill_rule(fill_props.fill_rule);
    }

    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        let (paint_type, paint_transform) = self.convert_paint(paint, false);
        self.ctx
            .fill_path(path, paint_type, paint_transform, self.cur_mask.clone());
    }

    fn draw_rgba_image(
        &mut self,
        image: hayro_interpret::RgbData,
        alpha: Option<hayro_interpret::LumaData>,
    ) {
        if let Some(ref mask) = self.cur_mask {
            self.ctx.push_layer(None, None, Some(mask.clone()));
        }

        self.ctx.set_anti_aliasing(false);
        self.draw_image(image, alpha, false);
        self.ctx.set_anti_aliasing(true);

        if self.cur_mask.is_some() {
            self.ctx.pop_layer();
        }
    }

    fn draw_stencil_image(&mut self, stencil: LumaData, paint: &Paint) {
        self.ctx.set_anti_aliasing(false);
        self.ctx.push_layer(None, Some(1.0), self.cur_mask.clone());
        let old_rule = self.ctx.fill_rule;
        self.set_fill_properties(&FillProps {
            fill_rule: FillRule::NonZero,
        });
        let (converted_paint, paint_transform) = self.convert_paint(paint, false);
        self.ctx.fill_rect(
            &Rect::new(0.0, 0.0, stencil.width as f64, stencil.height as f64),
            converted_paint,
            paint_transform,
            None,
        );
        let rgb_data = RgbData {
            data: vec![0; stencil.width as usize * stencil.height as usize * 3],
            width: stencil.width,
            height: stencil.height,
            interpolate: stencil.interpolate,
        };
        self.draw_image(rgb_data, Some(stencil), true);
        self.ctx.pop_layer();

        self.set_fill_properties(&FillProps {
            fill_rule: old_rule,
        });
        self.ctx.set_anti_aliasing(true);
    }

    fn fill_glyph(&mut self, glyph: &Glyph<'_>, paint: &Paint) {
        match glyph {
            Glyph::Outline(o) => {
                let outline = o.glyph_transform * o.outline();
                self.fill_path(&outline, paint);
            }
            Glyph::Type3(s) => {
                s.interpret(self, paint);
            }
        }
    }

    fn stroke_glyph(&mut self, glyph: &Glyph<'_>, paint: &Paint) {
        match glyph {
            Glyph::Outline(o) => {
                let outline = o.glyph_transform * o.outline();
                self.stroke_path(&outline, paint);
            }
            Glyph::Type3(s) => {
                s.interpret(self, paint);
            }
        }
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.ctx.set_fill_rule(clip_path.fill);
        self.ctx.push_layer(Some(&clip_path.path), None, None)
    }

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask>) {
        self.ctx.push_layer(
            None,
            Some(opacity),
            // TODO: Deduplicate
            mask.map(|m| {
                let width = self.ctx.width as u16;
                let height = self.ctx.height as u16;

                self.soft_mask_cache
                    .entry(m.id())
                    .or_insert_with(|| draw_soft_mask(&m, width, height))
                    .clone()
            }),
        );
    }

    fn pop_clip_path(&mut self) {
        self.ctx.pop_layer();
    }

    fn pop_transparency_group(&mut self) {
        self.ctx.pop_layer();
    }

    fn set_soft_mask(&mut self, mask: Option<SoftMask>) {
        self.cur_mask = mask.map(|m| {
            let width = self.ctx.width as u16;
            let height = self.ctx.height as u16;

            self.soft_mask_cache
                .entry(m.id())
                .or_insert_with(|| draw_soft_mask(&m, width, height))
                .clone()
        });
    }
}

pub struct RenderSettings {
    pub x_scale: f32,
    pub y_scale: f32,
    pub width: Option<u16>,
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

fn draw_soft_mask(mask: &SoftMask, width: u16, height: u16) -> Mask {
    let mut renderer = Renderer {
        ctx: RenderContext::new(width, height),
        inside_pattern: false,
        cur_mask: None,
        soft_mask_cache: Default::default(),
    };
    mask.interpret(&mut renderer);
    let mut pix = Pixmap::new(width, height);
    renderer.ctx.render_to_pixmap(&mut pix);

    match mask.mask_type() {
        MaskType::Luminosity => Mask::new_luminance(&pix),
        MaskType::Alpha => Mask::new_alpha(&pix),
    }
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

    return Some(rendered);
}

pub(crate) fn min_factor(transform: &Affine) -> f32 {
    let scale_skew_transform = {
        let c = transform.as_coeffs();
        Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
    };

    let x_advance = scale_skew_transform * Point::new(1.0, 0.0);
    let y_advance = scale_skew_transform * Point::new(0.0, 1.0);

    x_advance
        .to_vec2()
        .length()
        .min(y_advance.to_vec2().length()) as f32
}
