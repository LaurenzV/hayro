use crate::encode::{Buffer, x_y_advances};
use crate::paint::{Image, PaintType};
use crate::pixmap::Pixmap;
use crate::render::RenderContext;
use hayro_interpret::cache::Cache;
use hayro_interpret::clip_path::ClipPath;
use hayro_interpret::context::Context;
use hayro_interpret::device::Device;
use hayro_interpret::font::Glyph;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::util::FloatExt;
use hayro_interpret::{FillProps, Paint, StencilImage, StrokeProps, interpret};
use hayro_syntax::document::page::{A4, Page, Rotation};
use hayro_syntax::pdf::Pdf;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageBuffer, ImageEncoder};
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use peniko::Fill;
use peniko::color::palette::css::WHITE;
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

struct Renderer(RenderContext, bool);

impl Renderer {
    fn draw_image(
        &mut self,
        image_data: Vec<u8>,
        mut width: u32,
        mut height: u32,
        is_stencil: bool,
        interpolate: bool,
    ) {
        let mut cur_transform = self.0.transform;

        let (x_scale, y_scale) = {
            let (x, y) = x_y_advances(&cur_transform);
            (x.length() as f32, y.length() as f32)
        };

        let image_data = if x_scale >= 1.0 && y_scale >= 1.0 {
            image_data
        } else {
            // Do subsampling to prevent aliasing artifacts.
            let new_width = (width as f32 * x_scale).ceil().max(1.0) as u32;
            let new_height = (height as f32 * y_scale).ceil().max(1.0) as u32;

            let image = DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(width, height, image_data.clone()).unwrap(),
            );
            let resized = image.resize_exact(new_width, new_height, FilterType::CatmullRom);

            let new_width = resized.width();
            let new_height = resized.height();
            let t_scale_x = width as f32 / new_width as f32;
            let t_scale_y = height as f32 / new_height as f32;

            cur_transform *= Affine::scale_non_uniform(t_scale_x as f64, t_scale_y as f64);
            self.0.set_transform(cur_transform);

            width = new_width;
            height = new_height;

            resized.to_rgba8().into_raw()
        };

        let mut buffer = Buffer::<4>::new_u8(image_data, width, height);
        buffer.premultiply();

        let image = Image {
            buffer: Arc::new(buffer),
            interpolate,
            is_stencil,
            is_pattern: false,
        };

        self.0.fill_rect(
            &Rect::new(0.0, 0.0, width as f64, height as f64),
            image.into(),
            self.0.transform,
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
                match p {
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

                        let mut renderer =
                            Renderer(RenderContext::new(pix_width, pix_height), true);
                        let mut initial_transform =
                            Affine::new([xs as f64, 0.0, 0.0, ys as f64, -bbox.x0, -bbox.y0]);
                        t.interpret(&mut renderer, initial_transform, is_stroke);
                        let mut pix = Pixmap::new(pix_width, pix_height);
                        renderer.0.render_to_pixmap(&mut pix);

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
        self.0.set_transform(affine);
    }

    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps) {
        // Best-effort attempt to ensure a line width of at least 1.
        let min_factor = min_factor(&self.0.transform);
        let mut line_width = stroke_props.line_width.max(0.01);
        let transformed_width = line_width * min_factor;

        // Only enforce line width if not inside of pattern.
        if transformed_width < 1.0 && !self.1 {
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

        self.0.set_stroke(stroke);
    }

    fn stroke_path(&mut self, path: &BezPath, paint: &Paint) {
        let (paint_type, paint_transform) = self.convert_paint(paint, true);
        self.0.stroke_path(path, paint_type, paint_transform);
    }

    fn set_fill_properties(&mut self, fill_props: &FillProps) {
        self.0.set_fill_rule(fill_props.fill_rule);
    }

    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        let (paint_type, paint_transform) = self.convert_paint(paint, false);
        self.0.fill_path(path, paint_type, paint_transform);
    }

    fn draw_rgba_image(&mut self, image: hayro_interpret::RgbaImage) {
        self.0.set_anti_aliasing(false);
        self.draw_image(
            image.image_data,
            image.width,
            image.height,
            false,
            image.interpolate,
        );
        self.0.set_anti_aliasing(true);
    }

    fn draw_stencil_image(&mut self, stencil: StencilImage, paint: &Paint) {
        self.0.set_anti_aliasing(false);
        self.push_transparency_group(1.0);
        let old_rule = self.0.fill_rule;
        self.set_fill_properties(&FillProps {
            fill_rule: Fill::NonZero,
        });
        let (converted_paint, paint_transform) = self.convert_paint(paint, false);
        self.0.fill_rect(
            &Rect::new(0.0, 0.0, stencil.width as f64, stencil.height as f64),
            converted_paint,
            paint_transform,
        );
        self.draw_image(
            stencil.stencil_data,
            stencil.width,
            stencil.height,
            true,
            stencil.interpolate,
        );
        self.pop_transparency_group();

        self.set_fill_properties(&FillProps {
            fill_rule: old_rule,
        });
        self.0.set_anti_aliasing(true);
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
        self.0.set_fill_rule(clip_path.fill);
        self.0.push_layer(Some(&clip_path.path), None, None, None)
    }

    fn push_transparency_group(&mut self, opacity: f32) {
        self.0.push_layer(None, None, Some(opacity), None)
    }

    fn pop_clip_path(&mut self) {
        self.0.pop_layer();
    }

    fn pop_transparency_group(&mut self) {
        self.0.pop_layer();
    }
}

pub fn render(page: &Page, scale: f32) -> Pixmap {
    let crop_box = page.crop_box().intersect(page.media_box());

    let (unscaled_width, unscaled_height) = if (crop_box.width() as f32).is_nearly_zero()
        || (crop_box.height() as f32).is_nearly_zero()
    {
        (A4.width(), A4.height())
    } else {
        (crop_box.width(), crop_box.height())
    };

    let (mut pix_width, mut pix_height) = (unscaled_width, unscaled_height);

    let rotation_transform = Affine::scale(scale as f64)
        * match page.rotation() {
            Rotation::None => Affine::IDENTITY,
            Rotation::Horizontal => {
                let t = Affine::rotate(90.0f64.to_radians())
                    * Affine::translate((0.0, -unscaled_height));
                std::mem::swap(&mut pix_width, &mut pix_height);

                t
            }
            Rotation::Flipped => {
                Affine::scale(-1.0) * Affine::translate((-unscaled_width, -unscaled_height))
            }
            Rotation::FlippedHorizontal => {
                let t = Affine::translate((0.0, unscaled_width))
                    * Affine::rotate(-90.0f64.to_radians());
                std::mem::swap(&mut pix_width, &mut pix_height);

                t
            }
        };

    let initial_transform = rotation_transform
        * Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, unscaled_height])
        * Affine::translate((-crop_box.x0, -crop_box.y0));

    let (scaled_width, scaled_height) = (
        (pix_width as f32 * scale) as f64,
        (pix_height as f32 * scale) as f64,
    );
    let (pix_width, pix_height) = (scaled_width.floor() as u16, scaled_height.floor() as u16);
    let mut state = Context::new(
        initial_transform,
        kurbo::Rect::new(0.0, 0.0, pix_width as f64, pix_height as f64),
        Cache::new(),
        page.xref(),
    );
    let mut device = Renderer(RenderContext::new(pix_width, pix_height), false);

    device.0.fill_rect(
        &Rect::new(0.0, 0.0, pix_width as f64, pix_height as f64),
        WHITE.into(),
        Affine::IDENTITY,
    );
    device.push_clip_path(&ClipPath {
        path: initial_transform * crop_box.to_path(0.1),
        fill: Fill::NonZero,
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
    device.0.render_to_pixmap(&mut pixmap);
    pixmap
}

pub fn render_png(
    pdf: &Pdf,
    scale: f32,
    range: Option<RangeInclusive<usize>>,
) -> Option<Vec<Vec<u8>>> {
    if let Some(pages) = pdf.pages() {
        let rendered = pages
            .iter()
            .enumerate()
            .flat_map(|(idx, page)| {
                if range.clone().is_some_and(|range| !range.contains(&idx)) {
                    return None;
                }

                let pixmap = render(page, scale);

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

    None
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
