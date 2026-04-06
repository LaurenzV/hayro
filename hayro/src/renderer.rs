use crate::{RenderCache, derive_settings};
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer, images::Image as FirImage};
use hayro_interpret::encode::EncodedShadingPattern;
use hayro_interpret::font::Glyph;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Device, FillRule, GlyphDrawMode, ImageData, LumaData, MaskType,
    Paint, PathDrawMode, RgbData, SoftMask, StrokeProps,
};
use kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use vello_cpu::color::palette::css::BLACK;
use vello_cpu::color::{AlphaColor, PremulRgba8, Srgb};
use vello_cpu::peniko::{Compose, Fill, ImageQuality, ImageSampler, Mix};
use vello_cpu::{
    Image, ImageSource, Mask, PaintType, Pixmap, RenderContext, RenderSettings, peniko,
};

pub(crate) struct Renderer {
    pub(crate) ctx: RenderContext,
    pub(crate) inside_pattern: bool,
    pub(crate) soft_mask_cache: HashMap<u128, Mask>,
    pub(crate) outline_cache: Rc<std::cell::RefCell<HashMap<u128, Rc<BezPath>>>>,
    pub(crate) cur_mask: Option<Mask>,
    pub(crate) cur_blend_mode: BlendMode,
    pub(crate) in_type3_glyph: bool,
}

impl Renderer {
    pub(crate) fn new(
        width: u16,
        height: u16,
        settings: RenderSettings,
        cache: &RenderCache<'_>,
    ) -> Self {
        Self {
            ctx: RenderContext::new_with(width, height, settings),
            inside_pattern: false,
            soft_mask_cache: HashMap::default(),
            outline_cache: cache.outline_cache.clone(),
            cur_mask: None,
            cur_blend_mode: BlendMode::default(),
            in_type3_glyph: false,
        }
    }

    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps, is_text: bool) {
        let threshold = if is_text { 0.25 } else { 1.0 };

        // Best-effort attempt to ensure a line width of at least 1.0, as required by the PDF
        // specification. If we are stroking text, we reduce the threshold as it will otherwise
        // lead to very bold-looking text at low resolutions.
        let min_factor = max_factor(self.ctx.transform());
        let mut line_width = stroke_props.line_width.max(0.01);
        let transformed_width = line_width * min_factor;

        // Only enforce line width if not inside of pattern or type 3 glyph.
        if transformed_width < threshold && !self.inside_pattern && !self.in_type3_glyph {
            line_width /= transformed_width;
            line_width *= threshold;
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

    fn draw_image_with_alpha_mask(&mut self, image_data: ImageData, alpha_data: LumaData) {
        let mask = {
            let transform = *self.ctx.transform()
                * Affine::scale_non_uniform(
                    image_data.width() as f64 / alpha_data.width as f64,
                    image_data.height() as f64 / alpha_data.height as f64,
                );
            let mut renderer = Self {
                ctx: RenderContext::new_with(
                    self.ctx.width(),
                    self.ctx.height(),
                    derive_settings(self.ctx.render_settings()),
                ),
                inside_pattern: false,
                soft_mask_cache: HashMap::default(),
                outline_cache: self.outline_cache.clone(),
                cur_mask: None,
                cur_blend_mode: BlendMode::default(),
                in_type3_glyph: false,
            };
            let mut mask_pix = Pixmap::new(self.ctx.width(), self.ctx.height());
            let rgb_data = ImageData::Rgb(RgbData {
                data: vec![0; alpha_data.width as usize * alpha_data.height as usize * 3],
                width: alpha_data.width,
                height: alpha_data.height,
                interpolate: alpha_data.interpolate,
                scale_factors: alpha_data.scale_factors,
            });
            renderer.ctx.set_transform(transform);
            // Note that there is a circle between `draw_image` and `draw_image_with_alpha_mask`,
            // but `draw_image_with_alpha_mask` is only called if the dimensions or interpolate
            // values between alpha_data and rgb_data don't match, which they do here.
            renderer.draw_image(rgb_data, Some(alpha_data));
            renderer.ctx.flush();
            renderer.ctx.render_to_pixmap(&mut mask_pix);
            Mask::new_alpha(&mask_pix)
        };

        self.ctx.push_mask_layer(mask);
        self.draw_image(image_data, None);
        self.ctx.pop_layer();
    }

    fn resize_image(
        data: Vec<u8>,
        src_width: u32,
        src_height: u32,
        new_width: u32,
        new_height: u32,
        pixel_type: PixelType,
    ) -> Vec<u8> {
        let src_image = FirImage::from_vec_u8(src_width, src_height, data, pixel_type).unwrap();
        let mut dst_image = FirImage::new(new_width, new_height, pixel_type);

        let mut resizer = Resizer::new();
        resizer
            .resize(
                &src_image,
                &mut dst_image,
                &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(
                    fast_image_resize::FilterType::CatmullRom,
                )),
            )
            .unwrap();

        dst_image.into_vec()
    }

    fn draw_image(&mut self, image_data: ImageData, alpha_data: Option<LumaData>) {
        let cur_transform = *self.ctx.transform();
        let mut additional_transform = Affine::IDENTITY;

        let (x_scale, y_scale) = {
            let (x, y) = x_y_advances(&cur_transform);
            (x.length() as f32, y.length() as f32)
        };
        let mut img_width = image_data.width();
        let mut img_height = image_data.height();
        let interpolate = image_data.interpolate();

        if let Some(a) = &alpha_data
            && (a.width != img_width || a.height != img_height || a.interpolate != interpolate)
        {
            return self.draw_image_with_alpha_mask(image_data, alpha_data.unwrap());
        }

        let mut quality = if interpolate {
            ImageQuality::Medium
        } else {
            ImageQuality::Low
        };

        let needs_resize = x_scale < 1.0 || y_scale < 1.0;
        let (new_width, new_height) = if needs_resize {
            let w = (img_width as f32 * x_scale)
                .ceil()
                .max(1.0)
                .min((u16::MAX / 2) as f32) as u32;
            let h = (img_height as f32 * y_scale)
                .ceil()
                .max(1.0)
                .min((u16::MAX / 2) as f32) as u32;
            if self.in_type3_glyph {
                quality = ImageQuality::High;
            }
            (w, h)
        } else {
            (img_width, img_height)
        };

        // For luma images without alpha, we can resize as single-channel and
        // expand to RGBA afterwards, which is ~4x faster.
        let mut rgba_data = if matches!(&image_data, ImageData::Luma(_)) && alpha_data.is_none() {
            // We cannot lift this up due to borrowing issues.
            let ImageData::Luma(luma) = image_data else {
                unreachable!()
            };

            let luma_data = if !needs_resize {
                luma.data
            } else {
                let resized = Self::resize_image(
                    luma.data,
                    img_width,
                    img_height,
                    new_width,
                    new_height,
                    PixelType::U8,
                );
                additional_transform = Affine::scale_non_uniform(
                    img_width as f64 / new_width as f64,
                    img_height as f64 / new_height as f64,
                );
                img_width = new_width;
                img_height = new_height;
                resized
            };

            luma_data
                .iter()
                .flat_map(|g| [*g, *g, *g, 255])
                .collect::<Vec<_>>()
        } else {
            let (rgb_data, alpha_data) = match image_data {
                ImageData::Rgb(rgb) => (rgb.data, alpha_data.map(|a| a.data)),
                ImageData::Luma(luma) => {
                    let rgb = luma
                        .data
                        .iter()
                        .flat_map(|g| [*g, *g, *g])
                        .collect::<Vec<_>>();
                    (rgb, alpha_data.map(|a| a.data))
                }
            };

            let rgba_data = match alpha_data {
                None => rgb_data
                    .chunks_exact(3)
                    .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
                    .collect::<Vec<_>>(),
                Some(alpha) => rgb_data
                    .chunks_exact(3)
                    .zip(alpha)
                    .flat_map(|(rgb, a)| [rgb[0], rgb[1], rgb[2], a])
                    .collect::<Vec<_>>(),
            };

            if !needs_resize {
                rgba_data
            } else {
                let resized = Self::resize_image(
                    rgba_data,
                    img_width,
                    img_height,
                    new_width,
                    new_height,
                    PixelType::U8x4,
                );
                additional_transform = Affine::scale_non_uniform(
                    img_width as f64 / new_width as f64,
                    img_height as f64 / new_height as f64,
                );
                img_width = new_width;
                img_height = new_height;
                resized
            }
        };

        let (chunks, _) = rgba_data.as_chunks_mut::<4>();

        for chunk in chunks {
            *chunk = AlphaColor::from_rgba8(chunk[0], chunk[1], chunk[2], chunk[3])
                .premultiply()
                .to_rgba8()
                .to_u8_array();
        }

        // The problem is that by default, when applying a bilinear or bicubic scaling, we will
        // sample pixels using an extend (pad/reflect/repeat). For glyphs, this is undesirable
        // as the glyphs will look very bold. Therefore, for glyphs it is more desirable to sample
        // a transparent pixel when reaching the border. Thus, we wrap glyphs in a transparent frame
        // of pixel width 2.
        if self.in_type3_glyph {
            let mut padded_image = vec![];
            padded_image.extend(vec![0; (4 * img_width as usize + 16) * 2]);

            for row in rgba_data.chunks_exact(img_width as usize * 4) {
                padded_image.extend([0; 8]);
                padded_image.extend(row);
                padded_image.extend([0; 8]);
            }

            padded_image.extend(vec![0; (4 * img_width as usize + 16) * 2]);
            img_width += 4;
            img_height += 4;
            additional_transform *= Affine::translate((-2.0, -2.0));

            rgba_data = padded_image;
        }

        let pixmap = Pixmap::from_parts(
            bytemuck::cast_vec(rgba_data),
            img_width as u16,
            img_height as u16,
        );

        self.with_blend(|r| {
            r.draw_pixmap(
                Arc::new(pixmap),
                quality,
                cur_transform * additional_transform,
            );
        });
    }

    // TODO: Remove this method once vello_cpu supports inline blends.
    fn with_blend(&mut self, op: impl FnOnce(&mut Self)) {
        let push = self.cur_blend_mode != BlendMode::default();
        if push {
            self.ctx
                .push_blend_layer(convert_blend_mode(self.cur_blend_mode));
        }

        op(self);

        if push {
            self.ctx.pop_layer();
        }
    }

    fn push_clip_path_inner(&mut self, clip_path: &BezPath, fill: FillRule) {
        let old_transform = *self.ctx.transform();

        self.ctx.set_fill_rule(convert_fill_rule(fill));
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.push_clip_path(clip_path);

        self.ctx.set_transform(old_transform);
    }

    fn draw_pixmap(&mut self, pixmap: Arc<Pixmap>, quality: ImageQuality, transform: Affine) {
        let (width, height) = (pixmap.width(), pixmap.height());
        let image = Image {
            image: ImageSource::Pixmap(pixmap),
            sampler: ImageSampler {
                x_extend: peniko::Extend::Pad,
                y_extend: peniko::Extend::Pad,
                quality,
                alpha: 1.0,
            },
        };

        self.ctx.set_transform(transform);
        self.ctx.set_paint(image);
        self.ctx
            .fill_rect(&Rect::new(0.0, 0.0, width as f64, height as f64));
    }

    #[must_use]
    fn set_paint(&mut self, paint: &Paint<'_>, path: &BezPath, is_stroke: bool) -> Option<BezPath> {
        let mut paint_transform = Affine::IDENTITY;
        let mut clip_path = None;

        let paint: PaintType = match paint.clone() {
            Paint::Color(c) => {
                let c = c.to_rgba().to_rgba8();
                AlphaColor::from_rgba8(c[0], c[1], c[2], c[3]).into()
            }
            Paint::Pattern(p) => {
                let path_transform = self.ctx.transform();

                match *p {
                    Pattern::Shading(s) => {
                        clip_path = s.shading.clip_path.clone();
                        let mut bbox = (*path_transform * path.clone()).bounding_box();

                        if is_stroke {
                            // Try to account for stroke in bbox.
                            let (a1, a2) = x_y_advances(path_transform);
                            let factor = a1.length().max(a2.length()) * self.ctx.stroke().width;
                            bbox = bbox.inflate(factor, factor);
                        }

                        bbox = bbox.intersect(Rect::new(
                            0.0,
                            0.0,
                            self.ctx.width() as f64,
                            self.ctx.height() as f64,
                        ));

                        let encoded = s.encode();
                        let (image, width, height, transform) =
                            render_shading_texture(bbox, &encoded);
                        paint_transform = path_transform.inverse() * transform;

                        let pixmap = Pixmap::from_parts(image, width as u16, height as u16);

                        let image = Image {
                            image: ImageSource::Pixmap(Arc::new(pixmap)),
                            sampler: ImageSampler {
                                x_extend: peniko::Extend::Repeat,
                                y_extend: peniko::Extend::Repeat,
                                quality: ImageQuality::Medium,
                                alpha: 1.0,
                            },
                        };

                        PaintType::Image(image)
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
                            let (x, y) = x_y_advances(&(t.matrix));
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

                        let mut renderer = Self {
                            ctx: RenderContext::new_with(
                                pix_width,
                                pix_height,
                                derive_settings(self.ctx.render_settings()),
                            ),
                            cur_mask: None,
                            inside_pattern: true,
                            soft_mask_cache: HashMap::default(),
                            outline_cache: self.outline_cache.clone(),
                            cur_blend_mode: BlendMode::default(),
                            in_type3_glyph: false,
                        };
                        let mut initial_transform = Affine::scale_non_uniform(xs as f64, ys as f64)
                            * Affine::translate((-bbox.x0, -bbox.y0));
                        t.interpret(&mut renderer, initial_transform, is_stroke);
                        let mut pix = Pixmap::new(pix_width, pix_height);
                        renderer.ctx.flush();
                        renderer.ctx.render_to_pixmap(&mut pix);

                        // TODO: Fix these
                        if x_step < 0.0 {
                            initial_transform *=
                                Affine::new([-1.0, 0.0, 0.0, 1.0, scaled_width as f64, 0.0]);
                        }

                        if y_step < 0.0 {
                            initial_transform *=
                                Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, scaled_height as f64]);
                        }

                        paint_transform =
                            path_transform.inverse() * t.matrix * initial_transform.inverse();

                        let image = Image {
                            image: ImageSource::Pixmap(Arc::new(pix)),
                            sampler: ImageSampler {
                                x_extend: peniko::Extend::Repeat,
                                y_extend: peniko::Extend::Repeat,
                                quality: ImageQuality::Medium,
                                alpha: 1.0,
                            },
                        };

                        PaintType::Image(image)
                    }
                }
            }
        };

        self.ctx.set_paint_transform(paint_transform);
        self.ctx.set_paint(paint);

        clip_path
    }

    fn stroke_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'_>,
        stroke_props: &StrokeProps,
        is_text: bool,
    ) {
        self.ctx.set_transform(transform);
        self.set_stroke_properties(stroke_props, is_text);

        let clip_path = self.set_paint(paint, path, true);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, FillRule::NonZero);
        }
        self.with_blend(|r| {
            r.ctx.stroke_path(path);
        });
        if clip_path.is_some() {
            self.ctx.pop_clip_path();
        }
    }

    fn fill_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'_>,
        fill_rule: FillRule,
    ) {
        self.ctx.set_fill_rule(convert_fill_rule(fill_rule));
        self.ctx.set_transform(transform);

        let clip_path = self.set_paint(paint, path, false);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, fill_rule);
        }

        self.with_blend(|r| {
            r.ctx.fill_path(path);
        });

        if clip_path.is_some() {
            self.ctx.pop_clip_path();
        }
    }

    fn fill_glyph<'a>(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
    ) {
        match glyph {
            Glyph::Outline(o) => {
                let base_outline = self.cached_outline(o);

                self.fill_path(
                    base_outline.as_ref(),
                    transform * glyph_transform,
                    paint,
                    FillRule::NonZero,
                );
            }
            Glyph::Type3(s) => {
                self.in_type3_glyph = true;
                self.with_blend(|r| {
                    s.interpret(r, transform, glyph_transform, paint);
                });
                self.in_type3_glyph = false;
            }
        }
    }

    fn stroke_glyph<'a>(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        stroke_props: &StrokeProps,
    ) {
        match glyph {
            Glyph::Outline(o) => {
                let base_outline = self.cached_outline(o);

                self.stroke_path(
                    &(glyph_transform * base_outline.as_ref().clone()),
                    transform,
                    paint,
                    stroke_props,
                    true,
                );
            }
            Glyph::Type3(s) => {
                self.with_blend(|r| {
                    s.interpret(r, transform, glyph_transform, paint);
                });
            }
        }
    }

    fn cached_outline(&self, glyph: &hayro_interpret::font::OutlineGlyph) -> Rc<BezPath> {
        let id = glyph.identifier().cache_key();

        if let Some(path) = self.outline_cache.borrow().get(&id) {
            return path.clone();
        }

        let path = Rc::new(glyph.outline());
        self.outline_cache.borrow_mut().insert(id, path.clone());
        path
    }
}

impl<'a> Device<'a> for Renderer {
    fn draw_image(&mut self, image: hayro_interpret::Image<'a, '_>, mut transform: Affine) {
        self.ctx.set_paint_transform(Affine::IDENTITY);
        self.ctx.set_aliasing_threshold(Some(1));

        let target_width = (transform * Point::new(image.width() as f64, 0.0))
            .to_vec2()
            .length()
            .ceil() as u32;
        let target_height = (transform * Point::new(0.0, image.height() as f64))
            .to_vec2()
            .length()
            .ceil() as u32;

        match image {
            hayro_interpret::Image::Stencil(s) => {
                s.with_stencil(
                    |stencil, paint| {
                        transform *= Affine::scale_non_uniform(
                            stencil.scale_factors.0 as f64,
                            stencil.scale_factors.1 as f64,
                        );

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
                                self.ctx.set_transform(transform);
                                if push_layer {
                                    self.ctx.push_layer(
                                        None,
                                        Some(convert_blend_mode(self.cur_blend_mode)),
                                        Some(alpha as f32 / 255.0),
                                        None,
                                        None,
                                    );
                                }
                                let old_rule = *self.ctx.fill_rule();
                                self.ctx.set_fill_rule(Fill::NonZero);

                                let rgb_data = ImageData::Rgb(RgbData {
                                    data: rgb_bytes,
                                    width: stencil.width,
                                    height: stencil.height,
                                    interpolate: stencil.interpolate,
                                    scale_factors: stencil.scale_factors,
                                });
                                self.draw_image(rgb_data, Some(stencil));

                                if push_layer {
                                    self.ctx.pop_layer();
                                }

                                self.ctx.set_fill_rule(old_rule);
                            }
                            Paint::Pattern(_) => {
                                let (width, height) = (self.ctx.width(), self.ctx.height());
                                let stencil_rect = Rect::new(
                                    0.0,
                                    0.0,
                                    stencil.width as f64,
                                    stencil.height as f64,
                                );
                                let mask_pix = {
                                    let rgb_bytes = ImageData::Rgb(RgbData {
                                        data: vec![
                                            255;
                                            stencil.width as usize
                                                * stencil.height as usize
                                                * 3
                                        ],
                                        width: stencil.width,
                                        height: stencil.height,
                                        interpolate: stencil.interpolate,
                                        scale_factors: stencil.scale_factors,
                                    });
                                    let mut sub_renderer = Self {
                                        ctx: RenderContext::new_with(
                                            width,
                                            height,
                                            derive_settings(self.ctx.render_settings()),
                                        ),
                                        inside_pattern: false,
                                        soft_mask_cache: HashMap::default(),
                                        outline_cache: self.outline_cache.clone(),
                                        cur_mask: None,
                                        cur_blend_mode: BlendMode::default(),
                                        in_type3_glyph: false,
                                    };
                                    let mut sub_pix = Pixmap::new(width, height);
                                    sub_renderer.ctx.set_transform(transform);
                                    sub_renderer.draw_image(rgb_bytes, Some(stencil));
                                    sub_renderer.ctx.flush();
                                    sub_renderer.ctx.render_to_pixmap(&mut sub_pix);
                                    sub_pix
                                };

                                self.ctx.push_layer(
                                    None,
                                    Some(convert_blend_mode(self.cur_blend_mode)),
                                    None,
                                    Some(Mask::new_luminance(&mask_pix)),
                                    None,
                                );
                                self.ctx.set_transform(transform);

                                let clip_path =
                                    self.set_paint(paint, &stencil_rect.to_path(0.1), true);
                                if let Some(clip_path) = clip_path.as_ref() {
                                    self.push_clip_path_inner(clip_path, FillRule::NonZero);
                                }
                                self.ctx.fill_rect(&stencil_rect);
                                if clip_path.is_some() {
                                    self.ctx.pop_clip_path();
                                }

                                self.ctx.pop_layer();
                            }
                        };
                    },
                    Some((target_width, target_height)),
                );
            }
            hayro_interpret::Image::Raster(r) => {
                r.with_rgba(
                    |image, alpha| {
                        let (sx, sy) = image.scale_factors();
                        transform *= Affine::scale_non_uniform(sx as f64, sy as f64);
                        self.ctx.set_transform(transform);
                        self.with_blend(|r| {
                            r.draw_image(image, alpha);
                        });
                    },
                    Some((target_width, target_height)),
                );
            }
        }

        self.ctx.set_aliasing_threshold(None);
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.push_clip_path_inner(&clip_path.path, clip_path.fill);
    }

    fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'_>>,
        blend_mode: BlendMode,
    ) {
        let settings = *self.ctx.render_settings();
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
                    .or_insert_with(|| draw_soft_mask(&m, settings, width, height))
                    .clone()
            }),
            None,
        );
    }

    fn pop_clip_path(&mut self) {
        self.ctx.pop_clip_path();
    }

    fn pop_transparency_group(&mut self) {
        self.ctx.pop_layer();
    }

    fn set_soft_mask(&mut self, mask: Option<SoftMask<'_>>) {
        let settings = *self.ctx.render_settings();
        self.cur_mask = mask.map(|m| {
            let width = self.ctx.width();
            let height = self.ctx.height();

            self.soft_mask_cache
                .entry(m.cache_key())
                .or_insert_with(|| draw_soft_mask(&m, settings, width, height))
                .clone()
        });
        if let Some(mask) = self.cur_mask.clone() {
            self.ctx.set_mask(mask);
        } else {
            self.ctx.reset_mask();
        }
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
                Self::fill_path(self, path, transform, paint, *f);
            }
            PathDrawMode::Stroke(s) => {
                Self::stroke_path(self, path, transform, paint, s, false);
            }
        }
    }

    fn draw_rect(
        &mut self,
        rect: &Rect,
        transform: Affine,
        paint: &Paint<'_>,
        draw_mode: &PathDrawMode,
    ) {
        let path = rect.to_path(0.1);
        match draw_mode {
            PathDrawMode::Fill(fill_rule) => {
                self.ctx.set_fill_rule(convert_fill_rule(*fill_rule));
                self.ctx.set_transform(transform);

                let clip_path = self.set_paint(paint, &path, false);
                if let Some(clip_path) = clip_path.as_ref() {
                    self.push_clip_path_inner(clip_path, *fill_rule);
                }

                self.with_blend(|r| {
                    r.ctx.fill_rect(rect);
                });

                if clip_path.is_some() {
                    self.ctx.pop_clip_path();
                }
            }
            PathDrawMode::Stroke(s) => {
                Self::stroke_path(self, &path, transform, paint, s, false);
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
                Self::fill_glyph(self, glyph, transform, glyph_transform, paint);
            }
            GlyphDrawMode::Stroke(s) => {
                Self::stroke_glyph(self, glyph, transform, glyph_transform, paint, s);
            }
            GlyphDrawMode::Invisible => {
                // Don't render invisible text for visual output
            }
        }
    }

    fn set_blend_mode(&mut self, blend_mode: BlendMode) {
        self.cur_blend_mode = blend_mode;
    }
}

// TODO: Deduplicate with hayro-svg?
fn render_shading_texture(
    path_bbox: Rect,
    shading_pattern: &EncodedShadingPattern,
) -> (Vec<PremulRgba8>, u32, u32, Affine) {
    let base_width = (path_bbox.width() as f32).max(1.0);
    let base_height = (path_bbox.height() as f32).max(1.0);

    let width = (base_width).ceil() as u32;
    let height = (base_height).ceil() as u32;

    let (x_advance, y_advance) = x_y_advances(&shading_pattern.base_transform);

    let mut buf = vec![PremulRgba8::from_u32(0); width as usize * height as usize];
    let mut start_point = shading_pattern.base_transform
        * Affine::translate((0.5, 0.5))
        * Point::new(path_bbox.x0, path_bbox.y0);

    for row in buf.chunks_exact_mut(width as usize) {
        let mut point = start_point;

        for pixel in row {
            let sample = shading_pattern.sample(point);
            *pixel = AlphaColor::<Srgb>::new(sample).premultiply().to_rgba8();

            point += x_advance;
        }

        start_point += y_advance;
    }

    (
        buf,
        width,
        height,
        Affine::translate((path_bbox.x0, path_bbox.y0)),
    )
}

fn draw_soft_mask(mask: &SoftMask<'_>, settings: RenderSettings, width: u16, height: u16) -> Mask {
    let mut renderer = Renderer {
        ctx: RenderContext::new_with(width, height, derive_settings(&settings)),
        inside_pattern: false,
        cur_mask: None,
        soft_mask_cache: HashMap::default(),
        outline_cache: Rc::new(std::cell::RefCell::new(HashMap::new())),
        cur_blend_mode: BlendMode::default(),
        in_type3_glyph: false,
    };

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
    renderer.ctx.render_to_pixmap(&mut pix);

    let mut rendered_mask = match mask.mask_type() {
        MaskType::Luminosity => Mask::new_luminance(&pix),
        MaskType::Alpha => Mask::new_alpha(&pix),
    };

    if let Some(transfer_function) = mask.transfer_function() {
        let mut map = Vec::new();

        for y in 0..rendered_mask.height() {
            for x in 0..rendered_mask.width() {
                map.push(
                    (transfer_function.apply(rendered_mask.sample(x, y) as f32 / 255.0) * 255.0
                        + 0.5) as u8,
                );
            }
        }

        rendered_mask = Mask::from_parts(map, rendered_mask.width(), rendered_mask.height());
    }

    rendered_mask
}

pub(crate) fn max_factor(transform: &Affine) -> f32 {
    let scale_skew_transform = {
        let c = transform.as_coeffs();
        Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
    };

    let x_advance = scale_skew_transform * Point::new(1.0, 0.0);
    let y_advance = scale_skew_transform * Point::new(0.0, 1.0);

    x_advance
        .to_vec2()
        .length()
        .max(y_advance.to_vec2().length()) as f32
}

pub(crate) fn x_y_advances(transform: &Affine) -> (Vec2, Vec2) {
    let scale_skew_transform = {
        let c = transform.as_coeffs();
        Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
    };

    let x_advance = scale_skew_transform * Point::new(1.0, 0.0);
    let y_advance = scale_skew_transform * Point::new(0.0, 1.0);

    (
        Vec2::new(x_advance.x, x_advance.y),
        Vec2::new(y_advance.x, y_advance.y),
    )
}

fn convert_fill_rule(fill_rule: FillRule) -> Fill {
    match fill_rule {
        FillRule::NonZero => Fill::NonZero,
        FillRule::EvenOdd => Fill::EvenOdd,
    }
}

fn convert_blend_mode(blend_mode: BlendMode) -> peniko::BlendMode {
    let mix = match blend_mode {
        BlendMode::Normal => Mix::Normal,
        BlendMode::Multiply => Mix::Multiply,
        BlendMode::Screen => Mix::Screen,
        BlendMode::Overlay => Mix::Overlay,
        BlendMode::Darken => Mix::Darken,
        BlendMode::Lighten => Mix::Lighten,
        BlendMode::ColorDodge => Mix::ColorDodge,
        BlendMode::ColorBurn => Mix::ColorBurn,
        BlendMode::HardLight => Mix::HardLight,
        BlendMode::SoftLight => Mix::SoftLight,
        BlendMode::Difference => Mix::Difference,
        BlendMode::Exclusion => Mix::Exclusion,
        BlendMode::Hue => Mix::Hue,
        BlendMode::Saturation => Mix::Saturation,
        BlendMode::Color => Mix::Color,
        BlendMode::Luminosity => Mix::Luminosity,
    };

    peniko::BlendMode::new(mix, Compose::SrcOver)
}
