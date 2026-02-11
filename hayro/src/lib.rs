/*!
A crate for rendering PDF files.

This crate allows you to render pages of a PDF file into bitmaps. It is supposed to be relatively
lightweight, since we do not have any dependencies on the GPU. All the rendering happens on the CPU.

The ultimate goal of this crate is to be a *feature-complete* and *performant* PDF rasterizer.
With that said, we are currently still very far away from reaching that goal: So far, no effort
has been put into performance optimizations, as we are still working on implementing missing features.
However, this crate is currently the most comprehensive and feature-complete
implementation of a PDF rasterizer in pure Rust. This claim is supported by the fact that we currently
include over 1000 PDF files in our regression test suite. The majority of those have been scraped
from the `pdf.js` and `PDFBOX` test suites and therefore represent a very large and diverse sample
of PDF files.

As mentioned, there are still some serious limitations, including lack of support for
encrypted/password-protected PDF files, blending and isolation, knockout groups as well as a range
of smaller features such as color key masking. But you should be able to render the vast majority
of PDF files without too many issues.

## Safety
This crate forbids unsafe code via a crate-level attribute.

## Examples
For usage examples, see the [example](https://github.com/LaurenzV/hayro/tree/master/hayro/examples) in
the GitHub repository.

## Cargo features
This crate has one optional feature:
- `embed-fonts`: See the description of [`hayro-interpret`](https://docs.rs/hayro-interpret/latest/hayro_interpret/#cargo-features) for more information.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use crate::renderer::Renderer;
use hayro_interpret::Device;
use hayro_interpret::FillRule;
use hayro_interpret::InterpreterSettings;
use hayro_interpret::hayro_syntax::Pdf;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::util::{PageExt, RectExt};
use hayro_interpret::{BlendMode, Context};
use hayro_interpret::{ClipPath, interpret_page};
use kurbo::{Affine, Rect, Shape};
use std::ops::RangeInclusive;

pub use hayro_interpret;
pub use hayro_interpret::hayro_syntax;
pub use vello_cpu;

use vello_cpu::color::AlphaColor;
use vello_cpu::color::Srgb;
use vello_cpu::color::palette::css::TRANSPARENT;
use vello_cpu::color::palette::css::WHITE;
use vello_cpu::{Level, Pixmap, RenderMode};

mod renderer;

/// Settings to apply during rendering.
#[derive(Clone, Copy)]
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
    /// The background color. Determines the color of the base
    /// rectangle during rendering to a pixmap.
    pub bg_color: AlphaColor<Srgb>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            x_scale: 1.0,
            y_scale: 1.0,
            width: None,
            height: None,
            bg_color: TRANSPARENT,
        }
    }
}

/// Render the page with the given settings to a pixmap.
pub fn render(
    page: &Page<'_>,
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

    let vc_settings = vello_cpu::RenderSettings {
        level: Level::new(),
        num_threads: 0,
        render_mode: RenderMode::OptimizeSpeed,
    };

    let mut device = Renderer::new(pix_width, pix_height, vc_settings);

    device.ctx.set_paint(render_settings.bg_color);
    device
        .ctx
        .fill_rect(&Rect::new(0.0, 0.0, pix_width as f64, pix_height as f64));
    let mut clip_path = page.intersected_crop_box().to_kurbo().to_path(0.1);
    clip_path.apply_affine(initial_transform);
    device.push_clip_path(&ClipPath {
        path: clip_path,
        fill: FillRule::NonZero,
    });

    device.push_transparency_group(1.0, None, BlendMode::Normal);
    interpret_page(page, &mut state, &mut device);

    device.pop_transparency_group();

    device.pop_clip_path();

    let mut pixmap = Pixmap::new(pix_width, pix_height);
    device.ctx.render_to_pixmap(&mut pixmap);

    pixmap
}

use vello::wgpu;

/// Render the page with the given settings to a wgpu texture
#[cfg(feature = "gpu")]
pub fn render_gpu(
    page: &Page<'_>,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture_view: &wgpu::TextureView,
) -> Result<(), vello::Error> {
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

    let render_opts = vello::RendererOptions {
        ..Default::default()
    };

    let renderer = vello::Renderer::new(device, render_opts)?;

    let scene = vello::Scene::new();

    struct GpuRenderer {
        scene: vello::Scene,
        renderer: vello::Renderer,
        glyph_cache: std::collections::HashMap<u128, BezPath>,
        width: f32,
        height: f32,
        cur_blend_mode: BlendMode,
    }

    use hayro_interpret::{SoftMask, Paint, PathDrawMode, font::Glyph, GlyphDrawMode};
    use kurbo::BezPath;
    impl <'a> Device<'a> for GpuRenderer {
        fn draw_image(&mut self, image: hayro_interpret::Image<'a, '_>, mut transform: Affine) {
            let target_width = (transform * kurbo::Point::new(image.width() as f64, 0.0))
                .to_vec2()
                .length()
                .ceil() as u32;
            let target_height = (transform * kurbo::Point::new(0.0, image.height() as f64))
                .to_vec2()
                .length()
                .ceil() as u32;
            match image {
                hayro_interpret::Image::Stencil(s) => {
                    s.with_stencil(|stencil, paint| {
                        match paint {
                            Paint::Color(c) => {
                                let color = c.to_rgba().to_rgba8();
                                let (rgb_bytes, alpha) = (
                                    stencil
                                        .data
                                        .iter()
                                        .flat_map(|_| [color[0], color[1], color[2]])
                                        .collect::<Vec<u8>>(),
                                    color[3],
                                );

                                let push_layer =
                                    alpha != 255 || self.cur_blend_mode != BlendMode::default();

                                let style = vello::peniko::StyleRef::Fill(vello::peniko::Fill::NonZero);
                                let blend_mode = renderer::convert_blend_mode(self.cur_blend_mode);

                                if push_layer {
                                    self.scene.push_layer(
                                        style,
                                        blend_mode,
                                        alpha as f32 / 255.0,
                                        Affine::IDENTITY,
                                        &Rect::new(0.0, 0.0, stencil.width as f64, stencil.height as f64)
                                     )
                                }

                                let rgb_data = hayro_interpret::RgbData {
                                    data: rgb_bytes,
                                    width: stencil.width,
                                    height: stencil.height,
                                    interpolate: stencil.interpolate,
                                    scale_factors: stencil.scale_factors,
                                };
                                self.draw_image_data(rgb_data, Some(stencil), transform);

                                if push_layer {
                                    self.scene.pop_layer();
                                }

                            }
                            Paint::Pattern(p) => {
                                todo!("{p:?}");
                            }
                        }
                    }, Some((target_width, target_height)));
                }
                hayro_interpret::Image::Raster(r) => {
                    r.with_rgba(
                        |rgb, alpha| {
                            transform *= Affine::scale_non_uniform(
                                rgb.scale_factors.0 as f64,
                                rgb.scale_factors.1 as f64,
                            );

                            let push = self.cur_blend_mode != BlendMode::default();
                            if push {
                                let blend = renderer::convert_blend_mode(self.cur_blend_mode);
                                let style = vello::peniko::StyleRef::Fill(vello::peniko::Fill::NonZero);

                                self
                                    .scene
                                    .push_layer(
                                        style,
                                        blend,
                                        1.,
                                        Affine::IDENTITY,
                                        &Rect::new(0.0, 0.0, rgb.width as f64, rgb.height as f64)
                                    )
                            }

                            self.draw_image_data(rgb, alpha, transform);

                            if push {
                                self.scene.pop_layer();
                            }
                        },
                        Some((target_width, target_height)),
                    );
                }
            }
        }

        fn push_clip_path(&mut self, clip_path: &ClipPath) {
            let fill = &clip_path.fill;
            let path = &clip_path.path;

            let fill = renderer::convert_fill_rule(*fill);

            let style = vello::peniko::StyleRef::Fill(fill);
            let blend = vello::peniko::BlendMode::new(vello::peniko::Mix::Clip, vello::peniko::Compose::SrcOver);

            self.scene.push_clip_layer(
                style,
                Affine::IDENTITY,
                path,
              )
            /*
            self.scene.push_layer(
                style,
                blend,
                0.,
                Affine::IDENTITY,
                path,
             )
             */
        }

        fn push_transparency_group(
            &mut self,
            opacity: f32,
            mask: Option<SoftMask<'_>>,
            blend_mode: BlendMode,
        ) {
            if mask.is_some() {
                todo!("soft mask")
            }

            let blend = renderer::convert_blend_mode(blend_mode);
            let style = vello::peniko::StyleRef::Fill(vello::peniko::Fill::NonZero);

            /*
            self.scene.push_layer(
                style,
                blend,
                opacity,
                Affine::IDENTITY,
                &Rect::new(0.0, 0.0, self.width as f64, self.height as f64)
             );
             */
        }

        fn pop_clip_path(&mut self) {
            self.scene.pop_layer()
        }

        fn pop_transparency_group(&mut self) {
            self.scene.pop_layer()
        }

        fn set_soft_mask(&mut self, mask: Option<SoftMask<'_>>) {
            if let Some(mask) = mask {
                todo!("6: {mask:?}");
            }
            /*
            if let Some(mask) = mask {
                let style = vello::peniko::StyleRef::Fill(vello::peniko::Fill::NonZero);
                let blend = vello::peniko::BlendMode::new(vello::peniko::Mix::Clip, vello::peniko::Compose::SrcOver);

                self.scene.push_layer(
                    style,
                    blend,
                    0.,
                    Affine::IDENTITY,
                    &Rect::new(0.0, 0.0, self.width as f64, self.height as f64)
                );
                todo!("soft mask");
            } else {
                self.scene.pop_layer();
            }
            */
        }

        fn draw_path(
            &mut self,
            path: &BezPath,
            transform: Affine,
            paint: &Paint<'_>,
            draw_mode: &PathDrawMode,
        ) {
            match draw_mode {
                PathDrawMode::Fill(f) => {
                    self.draw_fill_path_data(path, transform, paint, *f);
                }
                PathDrawMode::Stroke(s) => {
                    self.draw_stroke_path_data(path, transform, paint, s);
                }
            }
        }

        fn draw_glyph(
            &mut self,
            glyph: &Glyph<'a>,
            transform: Affine,
            glyph_transform: Affine,
            paint: &Paint<'a>,
            draw_mode: &GlyphDrawMode,
        ) {
            match draw_mode {
                GlyphDrawMode::Fill => {
                    match glyph {
                        Glyph::Outline(o) => {
                            use hayro_interpret::CacheKey;
                            let id = o.identifier().cache_key();
                            let mut cache = std::mem::take(&mut self.glyph_cache);
                            let base_outline = cache
                                .entry(id)
                                .or_insert_with(|| o.outline());

                            self.draw_fill_path_data(base_outline, transform * glyph_transform, paint, FillRule::NonZero);
                            self.glyph_cache = cache;
                        }
                        Glyph::Type3(s) => {
                            s.interpret(self, transform, glyph_transform, paint);
                        }
                    }
                }
                GlyphDrawMode::Stroke(s) => {
                    match glyph {
                        Glyph::Outline(o) => {
                            use hayro_interpret::CacheKey;
                            let id = o.identifier().cache_key();
                            let base_outline = self
                                .glyph_cache
                                .entry(id)
                                .or_insert_with(|| o.outline())
                                .clone();

                            self.draw_stroke_path_data(&(glyph_transform * base_outline), transform, paint, s)
                        }
                        Glyph::Type3(s) => {
                            s.interpret(self, transform, glyph_transform, paint);
                        }
                    }
                }
                GlyphDrawMode::Invisible => (),
            }
        }

        fn set_blend_mode(&mut self, blend_mode: BlendMode) {
            self.cur_blend_mode = blend_mode;
        }
    }

    impl GpuRenderer {
        fn draw_image_data(&mut self, rgb_data: hayro_interpret::RgbData, alpha_data: Option<hayro_interpret::LumaData>, transform: Affine) {
            let mut rgb_width = rgb_data.width;
            let mut rgb_height = rgb_data.height;

            let rgba_data = match alpha_data {
                None => rgb_data
                    .data
                    .chunks_exact(3)
                    .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
                    .collect::<Vec<_>>(),
                Some(a) => {
                    if a.width != rgb_data.width
                        || a.height != rgb_data.height
                        || a.interpolate != rgb_data.interpolate
                    {
                        return self.draw_image_with_alpha_mask(rgb_data, a);
                    } else {
                        rgb_data
                            .data
                            .chunks_exact(3)
                            .zip(a.data)
                            .flat_map(|(rgb, a)| [rgb[0], rgb[1], rgb[2], a])
                            .collect::<Vec<_>>()
                    }
                }
            };

            let image_data = vello::peniko::ImageData {
                data: rgba_data.into(),
                format: vello::peniko::ImageFormat::Rgba8,
                alpha_type: vello::peniko::ImageAlphaType::Alpha,
                width: rgb_data.width,
                height: rgb_data.height,
            };

            /*
            let quality = if rgb_data.interpolate {
                vello::peniko::ImageQuality::Medium
            } else {
                vello::peniko::ImageQuality::Low
            };
            */
            let quality = vello::peniko::ImageQuality::High;
            let img_brush = vello::peniko::ImageBrush::new(image_data).with_quality(quality);

            self.scene.draw_image(img_brush.as_ref(), transform);
        }

        fn draw_image_with_alpha_mask(&mut self, rgb_data: hayro_interpret::RgbData, alpha_data: hayro_interpret::LumaData) {
            todo!("alpha mask")
        }

        fn draw_fill_path_data(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'_>, fill_rule: FillRule) {
            let fill = renderer::convert_fill_rule(fill_rule);

            let brush = paint_to_brush(paint, transform, path);

            self.scene.fill(
                fill,
                transform,
                &brush,
                None,
                path,
           )
        }

        fn draw_stroke_path_data(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'_>, s: &hayro_interpret::StrokeProps) {
            let stroke = kurbo::Stroke {
                width: s.line_width as f64,
                join: s.line_join,
                miter_limit: s.miter_limit as f64,
                start_cap: s.line_cap,
                end_cap: s.line_cap,
                dash_pattern: s.dash_array.iter().map(|n| *n as f64).collect(),
                dash_offset: s.dash_offset as f64,
            };

            let brush = paint_to_brush(paint, transform, path);

            self.scene.stroke(
                &stroke,
                transform,
                &brush,
                None,
                path,
            );
        }

    }

    fn paint_to_brush<'a>(paint: &Paint<'a>, transform: Affine, path: &BezPath) -> vello::peniko::Brush {
        match paint {
            Paint::Color(c) => {
                let c = c.to_rgba().to_rgba8();
                let a = AlphaColor::from_rgba8(c[0], c[1], c[2], c[3]).into();
                vello::peniko::Brush::Solid(a)
            }
            Paint::Pattern(p) => {
                use hayro_interpret::pattern::Pattern;
                match &**p {
                    Pattern::Shading(s) => {
                        let mut bbox = (transform * path.clone()).bounding_box();
                        let encoded = s.encode();
                        let (image, width, height, shading_transform) =
                            renderer::render_shading_texture(bbox, &encoded);

                        let paint_transform = transform.inverse() * shading_transform;

                        let bytes: Vec<u8> = bytemuck::cast_vec(image);


                        let image_data = vello::peniko::ImageData {
                            data: bytes.into(),
                            format: vello::peniko::ImageFormat::Rgba8,
                            alpha_type: vello::peniko::ImageAlphaType::AlphaPremultiplied,
                            width,
                            height,
                        };

                        let quality = vello::peniko::ImageQuality::High;
                        let img_brush = vello::peniko::ImageBrush::new(image_data).with_quality(quality);

                        vello::peniko::Brush::Image(img_brush)
                    }
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
                            let (x, y) = renderer::x_y_advances(&(t.matrix));
                            (x.length() as f32, y.length() as f32)
                        };
                        xs = xs.max(min_x_scale).min(max_x_scale);
                        ys = ys.max(min_y_scale).min(max_y_scale);

                        let x_step = xs * t.x_step;
                        let y_step = ys * t.y_step;

                        let scaled_width = bbox.width() as f32 * xs;
                        let scaled_height = bbox.height() as f32 * ys;
                        let pix_width = x_step.abs().round() as u16;
                        let pix_height = y_step.abs().round() as u16;

                        // FIXME: there might be a better way to do this
                        // so vello_cpu doesnt need to be involved
                        // - a smaller texture could be allocated 
                        //   then blitzed to a buffer through a 
                        //   seperate gpurenderer instance
                        let mut renderer = Renderer {
                            ctx: vello_cpu::RenderContext::new_with(
                                pix_width,
                                pix_height,
                                derive_settings(&Default::default()),
                            ),
                            cur_mask: None,
                            inside_pattern: true,
                            soft_mask_cache: Default::default(),
                            glyph_cache: Some(Default::default()),
                            cur_blend_mode: BlendMode::default(),
                            in_type3_glyph: false,
                        };
                        let mut initial_transform = Affine::scale_non_uniform(xs as f64, ys as f64)
                            * Affine::translate((-bbox.x0, -bbox.y0));
                        t.interpret(&mut renderer, initial_transform, false);
                        let mut pix = Pixmap::new(pix_width, pix_height);
                        renderer.ctx.flush();
                        renderer.ctx.render_to_pixmap(&mut pix);

                        let image_data = vello::peniko::ImageData {
                            data: bytemuck::cast_vec(pix.take()).into(),
                            format: vello::peniko::ImageFormat::Rgba8,
                            alpha_type: vello::peniko::ImageAlphaType::AlphaPremultiplied,
                            width: pix_width as _,
                            height: pix_height as _,
                        };

                        let quality = vello::peniko::ImageQuality::High;
                        let img_brush = vello::peniko::ImageBrush::new(image_data).with_quality(quality).with_extend(vello::peniko::Extend::Repeat);

                        vello::peniko::Brush::Image(img_brush)
                    }
                }
            }
        }
    }

    let mut renderer = GpuRenderer {
        scene,
        renderer,
        glyph_cache: Default::default(),
        width,
        height,
        cur_blend_mode: Default::default(),
    };

    interpret_page(page, &mut state, &mut renderer);

    let GpuRenderer {
        scene,
        mut renderer,
        ..
    } = renderer;

    renderer.render_to_texture(
        device,
        queue,
        &scene,
        texture_view,
        &vello::RenderParams {
            base_color: render_settings.bg_color,
            width: pix_width as _,
            height: pix_height as _,
            antialiasing_method: vello::AaConfig::Msaa16,
        },
    )?;
    Ok(())
}

// Just a convenience method for testing.
#[doc(hidden)]
pub fn render_pdf(
    pdf: &Pdf,
    scale: f32,
    settings: InterpreterSettings,
    range: Option<RangeInclusive<usize>>,
) -> Option<Vec<Pixmap>> {
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
                    bg_color: WHITE,
                    ..Default::default()
                },
            );

            Some(pixmap)
        })
        .collect();

    Some(rendered)
}

pub(crate) fn derive_settings(settings: &vello_cpu::RenderSettings) -> vello_cpu::RenderSettings {
    vello_cpu::RenderSettings {
        num_threads: 0,
        ..*settings
    }
}
