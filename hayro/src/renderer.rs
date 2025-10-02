use crate::derive_settings;
use hayro_interpret::encode::EncodedShadingPattern;
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_syntax::object::ObjectIdentifier;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::{
    CacheKey, ClipPath, Device, FillRule, GlyphDrawMode, LumaData, MaskType, Paint, PathDrawMode,
    RgbData, SoftMask, StrokeProps,
};
use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer};
use kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use std::collections::HashMap;
use std::sync::Arc;
use vello_cpu::color::palette::css::BLACK;
use vello_cpu::color::{AlphaColor, PremulRgba8, Srgb};
use vello_cpu::peniko::{Fill, ImageQuality, ImageSampler};
use vello_cpu::{
    Image, ImageSource, Mask, PaintType, Pixmap, RenderContext, RenderSettings, peniko,
};

pub(crate) struct Renderer {
    pub(crate) ctx: RenderContext,
    pub(crate) inside_pattern: bool,
    pub(crate) soft_mask_cache: HashMap<ObjectIdentifier, Mask>,
    pub(crate) glyph_cache: Option<HashMap<u128, BezPath>>,
    pub(crate) cur_mask: Option<Mask>,
}

impl Renderer {
    pub(crate) fn new(width: u16, height: u16, settings: RenderSettings) -> Self {
        Self {
            ctx: RenderContext::new_with(width, height, settings),
            inside_pattern: false,
            soft_mask_cache: Default::default(),
            glyph_cache: Some(HashMap::new()),
            cur_mask: None,
        }
    }

    fn set_stroke_properties(&mut self, stroke_props: &StrokeProps) {
        // Best-effort attempt to ensure a line width of at least 1.
        let min_factor = min_factor(self.ctx.transform());
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

    fn draw_image(&mut self, rgb_data: RgbData, alpha_data: Option<LumaData>) {
        let mut cur_transform = *self.ctx.transform();

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
            .map(|(rgb, a)| {
                AlphaColor::from_rgba8(rgb[0], rgb[1], rgb[2], a)
                    .premultiply()
                    .to_rgba8()
            })
            .collect::<Vec<_>>();

        let pixmap = Pixmap::from_parts(rgba_data, rgb_width as u16, rgb_height as u16);

        self.draw_pixmap(Arc::new(pixmap), interpolate, cur_transform);
    }

    fn push_clip_path_inner(&mut self, clip_path: &BezPath, fill: FillRule) {
        let old_transform = *self.ctx.transform();

        self.ctx.set_fill_rule(convert_fill_rule(fill));
        self.ctx.set_transform(Affine::IDENTITY);
        self.ctx.push_clip_path(clip_path);

        self.ctx.set_transform(old_transform);
    }

    fn draw_pixmap(&mut self, pixmap: Arc<Pixmap>, interpolate: bool, transform: Affine) {
        let quality = if !interpolate {
            ImageQuality::Low
        } else {
            ImageQuality::Medium
        };

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
    fn set_paint(&mut self, paint: &Paint, path: &BezPath, is_stroke: bool) -> Option<BezPath> {
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

                        let mut renderer = Renderer {
                            ctx: RenderContext::new_with(
                                pix_width,
                                pix_height,
                                derive_settings(self.ctx.render_settings()),
                            ),
                            cur_mask: None,
                            inside_pattern: true,
                            soft_mask_cache: Default::default(),
                            glyph_cache: Some(HashMap::new()),
                        };
                        let mut initial_transform =
                            Affine::new([xs as f64, 0.0, 0.0, ys as f64, -bbox.x0, -bbox.y0]);
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
        paint: &Paint,
        stroke_props: &StrokeProps,
    ) {
        self.ctx.set_transform(transform);
        self.set_stroke_properties(stroke_props);

        let clip_path = self.set_paint(paint, path, true);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, FillRule::NonZero);
        }
        self.ctx.stroke_path(path);
        if clip_path.is_some() {
            self.ctx.pop_clip_path();
        }
    }

    fn fill_path(&mut self, path: &BezPath, transform: Affine, paint: &Paint, fill_rule: FillRule) {
        self.ctx.set_fill_rule(convert_fill_rule(fill_rule));
        self.ctx.set_transform(transform);

        let clip_path = self.set_paint(paint, path, false);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, fill_rule);
        }

        self.ctx.fill_path(path);

        if clip_path.is_some() {
            self.ctx.pop_clip_path();
        }
    }

    fn fill_glyph(
        &mut self,
        glyph: &Glyph<'_>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint,
    ) {
        match glyph {
            Glyph::Outline(o) => {
                let id = o.identifier().cache_key();
                // Otherwise we run into lifetime issues.
                let mut cache = std::mem::take(&mut self.glyph_cache);
                let base_outline = cache
                    .as_mut()
                    .unwrap()
                    .entry(id)
                    .or_insert_with(|| o.outline());

                self.fill_path(
                    base_outline,
                    transform * glyph_transform,
                    paint,
                    FillRule::NonZero,
                );

                self.glyph_cache = cache;
            }
            Glyph::Type3(s) => {
                s.interpret(self, transform, glyph_transform, paint);
            }
        }
    }

    fn stroke_glyph(
        &mut self,
        glyph: &Glyph<'_>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint,
        stroke_props: &StrokeProps,
    ) {
        match glyph {
            Glyph::Outline(o) => {
                let id = o.identifier().cache_key();
                let base_outline = self
                    .glyph_cache
                    .as_mut()
                    .unwrap()
                    .entry(id)
                    .or_insert_with(|| o.outline())
                    .clone();

                self.stroke_path(
                    &(glyph_transform * base_outline),
                    transform,
                    paint,
                    stroke_props,
                );
            }
            Glyph::Type3(s) => {
                s.interpret(self, transform, glyph_transform, paint);
            }
        }
    }
}

impl<'a> Device<'a> for Renderer {
    fn draw_image(&mut self, image: hayro_interpret::Image<'a, '_>, transform: Affine) {
        self.ctx.set_paint_transform(Affine::IDENTITY);
        self.ctx.set_aliasing_threshold(Some(1));

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

                            let push_layer = alpha != 255;
                            self.ctx.set_transform(transform);
                            if push_layer {
                                self.ctx
                                    .push_layer(None, None, Some(alpha as f32 / 255.0), None);
                            }
                            let old_rule = *self.ctx.fill_rule();
                            self.ctx.set_fill_rule(Fill::NonZero);

                            let rgb_data = RgbData {
                                data: rgb_bytes,
                                width: stencil.width,
                                height: stencil.height,
                                interpolate: stencil.interpolate,
                            };
                            self.draw_image(rgb_data, Some(stencil));

                            if push_layer {
                                self.ctx.pop_layer();
                            }

                            self.ctx.set_fill_rule(old_rule);
                        }
                        Paint::Pattern(_) => {
                            let (width, height) = (self.ctx.width(), self.ctx.height());
                            let stencil_rect =
                                Rect::new(0.0, 0.0, stencil.width as f64, stencil.height as f64);
                            let mask_pix = {
                                let rgb_bytes = RgbData {
                                    data: vec![
                                        255;
                                        stencil.width as usize * stencil.height as usize * 3
                                    ],
                                    width: stencil.width,
                                    height: stencil.height,
                                    interpolate: stencil.interpolate,
                                };
                                let mut sub_renderer = Renderer::new(
                                    width,
                                    height,
                                    derive_settings(self.ctx.render_settings()),
                                );
                                let mut sub_pix = Pixmap::new(width, height);
                                sub_renderer.ctx.set_transform(transform);
                                sub_renderer.draw_image(rgb_bytes, Some(stencil));
                                sub_renderer.ctx.flush();
                                sub_renderer.ctx.render_to_pixmap(&mut sub_pix);
                                sub_pix
                            };

                            self.ctx.push_layer(
                                None,
                                None,
                                None,
                                Some(Mask::new_luminance(&mask_pix)),
                            );
                            self.ctx.set_transform(transform);

                            let clip_path = self.set_paint(paint, &stencil_rect.to_path(0.1), true);
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
                });
            }
            hayro_interpret::Image::Raster(r) => {
                r.with_rgba(|rgb, alpha| {
                    self.ctx.set_transform(transform);
                    self.draw_image(rgb, alpha);
                });
            }
        }

        self.ctx.set_aliasing_threshold(None);
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.push_clip_path_inner(&clip_path.path, clip_path.fill);
    }

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask>) {
        let settings = *self.ctx.render_settings();
        self.ctx.push_layer(
            None,
            None,
            Some(opacity),
            // TODO: Deduplicate
            mask.map(|m| {
                let width = self.ctx.width();
                let height = self.ctx.height();

                self.soft_mask_cache
                    .entry(m.id())
                    .or_insert_with(|| draw_soft_mask(&m, settings, width, height))
                    .clone()
            }),
        );
    }

    fn pop_clip_path(&mut self) {
        self.ctx.pop_clip_path();
    }

    fn pop_transparency_group(&mut self) {
        self.ctx.pop_layer();
    }

    fn set_soft_mask(&mut self, mask: Option<SoftMask>) {
        let settings = *self.ctx.render_settings();
        self.cur_mask = mask.map(|m| {
            let width = self.ctx.width();
            let height = self.ctx.height();

            self.soft_mask_cache
                .entry(m.id())
                .or_insert_with(|| draw_soft_mask(&m, settings, width, height))
                .clone()
        });
        self.ctx.set_mask(self.cur_mask.clone());
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
                Self::stroke_path(self, path, transform, paint, s);
            }
        }
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'_>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'_>,
        draw_mode: &GlyphDrawMode,
    ) {
        match draw_mode {
            GlyphDrawMode::Fill => {
                Self::fill_glyph(self, glyph, transform, glyph_transform, paint);
            }
            GlyphDrawMode::Stroke(s) => {
                Self::stroke_glyph(self, glyph, transform, glyph_transform, paint, s);
            }
        }
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

fn draw_soft_mask(
    mask: &SoftMask,
    settings: vello_cpu::RenderSettings,
    width: u16,
    height: u16,
) -> Mask {
    let mut renderer = Renderer {
        ctx: RenderContext::new_with(width, height, derive_settings(&settings)),
        inside_pattern: false,
        cur_mask: None,
        soft_mask_cache: Default::default(),
        glyph_cache: Some(HashMap::new()),
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
        renderer.ctx.push_layer(None, None, None, None);
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
