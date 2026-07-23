use crate::{Renderer, x_y_advances};
use fearless_simd::{Level, Select, Simd, SimdBase, SimdInto, mask8x32, u8x32, u16x32};
use hayro_interpret::{FillRule, ImageData, ImageDrawProps, LumaData, Paint, RgbData};
use kurbo::{Affine, Point, Rect};
use pic_scale::{
    ImageSize, ImageStore, ImageStoreMut, PicScaleError, Resampling, ResamplingFunction, Scaler,
};
use std::sync::Arc;
use vello_cpu::peniko::{Compose, Fill, ImageQuality, ImageSampler, Mix};
use vello_cpu::{Image, ImageSource, Mask, Pixmap, peniko};

// Previously, we used `CatmullRom`. The problem with that one is that it
// can have negative weights. If we pass a premultiplied buffer to
// `pic-scale`, it can happen that any of the RGB variants end up
// slightly larger than the alpha channel, leading to pixel artifacts when
// rendering. In order to avoid having to do another pass over the buffer
// to clamp, we instead use Hermite, which has similar quality but doesn't
// have this problem.
pub(super) const RESAMPLING_FUNCTION: ResamplingFunction = ResamplingFunction::Hermite;

#[derive(Clone, Copy)]
enum ImagePixelFormat {
    Luma,
    Rgb,
    PremultipliedRgba,
}

struct SolidColorImage {
    color: [u8; 3],
    width: u32,
    height: u32,
    interpolate: bool,
}

enum RenderImageData {
    Rgb(RgbData),
    Luma(LumaData),
    Solid(SolidColorImage),
}

impl From<ImageData> for RenderImageData {
    fn from(value: ImageData) -> Self {
        match value {
            ImageData::Rgb(rgb) => Self::Rgb(rgb),
            ImageData::Luma(luma) => Self::Luma(luma),
        }
    }
}

impl From<RgbData> for RenderImageData {
    fn from(value: RgbData) -> Self {
        Self::Rgb(value)
    }
}

impl RenderImageData {
    fn width(&self) -> u32 {
        match self {
            Self::Rgb(d) => d.width,
            Self::Luma(d) => d.width,
            Self::Solid(d) => d.width,
        }
    }

    fn height(&self) -> u32 {
        match self {
            Self::Rgb(d) => d.height,
            Self::Luma(d) => d.height,
            Self::Solid(d) => d.height,
        }
    }

    fn interpolate(&self) -> bool {
        match self {
            Self::Rgb(d) => d.interpolate,
            Self::Luma(d) => d.interpolate,
            Self::Solid(d) => d.interpolate,
        }
    }
}

impl Renderer<'_> {
    fn draw_image_with_alpha_mask(&mut self, image_data: RenderImageData, alpha_data: LumaData) {
        let img_width = image_data.width();
        let img_height = image_data.height();
        let image_transform = *self.ctx.transform();
        let mask_transform = image_transform
            * Affine::scale_non_uniform(
                img_width as f64 / alpha_data.width as f64,
                img_height as f64 / alpha_data.height as f64,
            );
        let mask_image = SolidColorImage {
            color: [0; 3],
            width: alpha_data.width,
            height: alpha_data.height,
            interpolate: alpha_data.interpolate,
        };

        self.ctx.push_layer(None, None, None, None, None);
        self.ctx.set_transform(image_transform);
        self.draw_image(image_data, None);

        self.ctx.push_layer(
            None,
            Some(peniko::BlendMode::new(Mix::Normal, Compose::DestIn)),
            None,
            None,
            None,
        );
        self.ctx.set_transform(mask_transform);
        // Note that there is a circle between `draw_image` and `draw_image_with_alpha_mask`,
        // but `draw_image_with_alpha_mask` is only called if the dimensions or interpolate
        // values between alpha_data and rgb_data don't match. Here we use a
        // `SolidColorImage` so it doesn't affect it.
        self.draw_image(RenderImageData::Solid(mask_image), Some(alpha_data));
        self.ctx.pop_layer();
        self.ctx.pop_layer();
        self.ctx.set_transform(image_transform);
    }

    fn resize_image_data(
        &self,
        data: Vec<u8>,
        src_width: u32,
        src_height: u32,
        new_width: u32,
        new_height: u32,
        pixel_format: ImagePixelFormat,
    ) -> Vec<u8> {
        match pixel_format {
            ImagePixelFormat::Luma => self.resize_image_data_impl::<1>(
                data,
                src_width,
                src_height,
                new_width,
                new_height,
                |scaler, source_size, target_size| {
                    scaler.plan_planar_resampling(source_size, target_size)
                },
            ),
            ImagePixelFormat::Rgb => self.resize_image_data_impl::<3>(
                data,
                src_width,
                src_height,
                new_width,
                new_height,
                |scaler, source_size, target_size| {
                    scaler.plan_rgb_resampling(source_size, target_size)
                },
            ),
            ImagePixelFormat::PremultipliedRgba => self.resize_image_data_impl::<4>(
                data,
                src_width,
                src_height,
                new_width,
                new_height,
                |scaler, source_size, target_size| {
                    scaler.plan_rgba_resampling(source_size, target_size, false)
                },
            ),
        }
    }

    fn resize_image_data_impl<const N: usize>(
        &self,
        data: Vec<u8>,
        src_width: u32,
        src_height: u32,
        new_width: u32,
        new_height: u32,
        plan: impl FnOnce(
            &Scaler,
            ImageSize,
            ImageSize,
        ) -> Result<Arc<Resampling<u8, N>>, PicScaleError>,
    ) -> Vec<u8> {
        let source_size = ImageSize::new(src_width as usize, src_height as usize);
        let target_size = ImageSize::new(new_width as usize, new_height as usize);
        let src = ImageStore::<u8, N>::from_slice(&data, src_width as usize, src_height as usize)
            .unwrap();
        let mut out = vec![0; new_width as usize * new_height as usize * N];
        let mut dst =
            ImageStoreMut::<u8, N>::from_slice(&mut out, new_width as usize, new_height as usize)
                .unwrap();
        let plan = plan(&self.global.scaler, source_size, target_size).unwrap();
        plan.resample(&src, &mut dst).unwrap();
        out
    }

    fn draw_image(&mut self, image_data: impl Into<RenderImageData>, alpha_data: Option<LumaData>) {
        let image_data = image_data.into();
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

        let has_alpha = alpha_data.is_some();
        let mut may_have_transparency = has_alpha;
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
        let mut needs_premultiplication = has_alpha;
        let mut rgba_data = if matches!(&image_data, RenderImageData::Solid(_)) && has_alpha {
            let RenderImageData::Solid(solid) = image_data else {
                unreachable!()
            };
            let alpha = alpha_data.unwrap();

            let alpha_data = if !needs_resize {
                alpha.data
            } else {
                let resized_alpha = self.resize_image_data(
                    alpha.data,
                    img_width,
                    img_height,
                    new_width,
                    new_height,
                    ImagePixelFormat::Luma,
                );
                additional_transform = Affine::scale_non_uniform(
                    img_width as f64 / new_width as f64,
                    img_height as f64 / new_height as f64,
                );
                img_width = new_width;
                img_height = new_height;
                resized_alpha
            };

            let mut out = Vec::with_capacity(img_width as usize * img_height as usize * 4);
            for a in alpha_data {
                out.extend_from_slice(&[solid.color[0], solid.color[1], solid.color[2], a]);
            }
            out
        } else if matches!(&image_data, RenderImageData::Luma(_)) && !has_alpha {
            // We cannot lift this up due to borrowing issues.
            let RenderImageData::Luma(luma) = image_data else {
                unreachable!()
            };

            let luma_data = if !needs_resize {
                luma.data
            } else {
                let resized = self.resize_image_data(
                    luma.data,
                    img_width,
                    img_height,
                    new_width,
                    new_height,
                    ImagePixelFormat::Luma,
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
        } else if matches!(&image_data, RenderImageData::Luma(_)) && has_alpha {
            let RenderImageData::Luma(luma) = image_data else {
                unreachable!()
            };
            let alpha = alpha_data.unwrap();

            let (luma_data, alpha_data) = if !needs_resize {
                (luma.data, alpha.data)
            } else {
                let resized_luma = self.resize_image_data(
                    luma.data,
                    img_width,
                    img_height,
                    new_width,
                    new_height,
                    ImagePixelFormat::Luma,
                );
                let resized_alpha = self.resize_image_data(
                    alpha.data,
                    img_width,
                    img_height,
                    new_width,
                    new_height,
                    ImagePixelFormat::Luma,
                );
                additional_transform = Affine::scale_non_uniform(
                    img_width as f64 / new_width as f64,
                    img_height as f64 / new_height as f64,
                );
                img_width = new_width;
                img_height = new_height;
                (resized_luma, resized_alpha)
            };

            let mut out = Vec::with_capacity(img_width as usize * img_height as usize * 4);
            for (g, a) in luma_data.iter().zip(alpha_data) {
                out.extend_from_slice(&[*g, *g, *g, a]);
            }
            out
        } else if matches!(&image_data, RenderImageData::Rgb(_)) && !has_alpha && needs_resize {
            let RenderImageData::Rgb(rgb) = image_data else {
                unreachable!()
            };

            let resized = self.resize_image_data(
                rgb.data,
                img_width,
                img_height,
                new_width,
                new_height,
                ImagePixelFormat::Rgb,
            );
            additional_transform = Affine::scale_non_uniform(
                img_width as f64 / new_width as f64,
                img_height as f64 / new_height as f64,
            );
            img_width = new_width;
            img_height = new_height;

            let mut out = Vec::with_capacity((img_width * img_height) as usize * 4);
            for px in resized.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        } else {
            let (rgb_data, alpha_data) = match image_data {
                RenderImageData::Rgb(rgb) => (rgb.data, alpha_data.map(|a| a.data)),
                RenderImageData::Luma(luma) => {
                    let rgb = luma
                        .data
                        .iter()
                        .flat_map(|g| [*g, *g, *g])
                        .collect::<Vec<_>>();
                    (rgb, alpha_data.map(|a| a.data))
                }
                RenderImageData::Solid(solid) => {
                    let mut rgb =
                        Vec::with_capacity(solid.width as usize * solid.height as usize * 3);
                    for _ in 0..solid.width as usize * solid.height as usize {
                        rgb.extend_from_slice(&solid.color);
                    }
                    (rgb, alpha_data.map(|a| a.data))
                }
            };

            let mut rgba_data = match alpha_data {
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
                premultiply_rgba(self.global.level, &mut rgba_data);

                needs_premultiplication = false;
                let resized = self.resize_image_data(
                    rgba_data,
                    img_width,
                    img_height,
                    new_width,
                    new_height,
                    ImagePixelFormat::PremultipliedRgba,
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

        if needs_premultiplication {
            premultiply_rgba(self.global.level, &mut rgba_data);
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
            may_have_transparency = true;

            rgba_data = padded_image;
        }

        let pixmap = Pixmap::from_parts_with_opacity(
            bytemuck::cast_vec(rgba_data),
            img_width as u16,
            img_height as u16,
            may_have_transparency,
        );

        self.draw_pixmap(
            Arc::new(pixmap),
            quality,
            cur_transform * additional_transform,
        );
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

    pub(super) fn draw_pdf_image<'a>(
        &mut self,
        image: hayro_interpret::Image<'a, '_>,
        props: ImageDrawProps<'a>,
    ) {
        self.apply_image_props(&props);
        let mut transform = props.transform;
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
                                let alpha = color[3];

                                let blend_mode = self.ctx.blend_mode();
                                let push_layer =
                                    alpha != 255 || blend_mode != peniko::BlendMode::default();
                                self.ctx.set_transform(transform);
                                if push_layer {
                                    self.ctx.push_layer(
                                        None,
                                        Some(blend_mode),
                                        Some(alpha as f32 / 255.0),
                                        None,
                                        None,
                                    );
                                }
                                let old_rule = *self.ctx.fill_rule();
                                self.ctx.set_fill_rule(Fill::NonZero);

                                self.draw_image(
                                    RenderImageData::Solid(SolidColorImage {
                                        color: [color[0], color[1], color[2]],
                                        width: stencil.width,
                                        height: stencil.height,
                                        interpolate: stencil.interpolate,
                                    }),
                                    Some(stencil),
                                );

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
                                    let mut sub_renderer = self.child(width, height);
                                    let mut sub_pix = Pixmap::new(width, height);
                                    sub_renderer.ctx.set_transform(transform);
                                    sub_renderer.draw_image(rgb_bytes, Some(stencil));
                                    sub_renderer.ctx.flush();
                                    let mut resources = vello_cpu::Resources::default();
                                    sub_renderer.ctx.render(&mut sub_pix, &mut resources);
                                    sub_pix
                                };

                                self.ctx.push_layer(
                                    None,
                                    Some(self.ctx.blend_mode()),
                                    None,
                                    Some(Mask::new_luminance(&mask_pix)),
                                    None,
                                );
                                self.ctx.set_transform(transform);

                                let clip_path = self.set_paint(paint, || stencil_rect, false);
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
                        self.draw_image(image, alpha);
                    },
                    Some((target_width, target_height)),
                );
            }
        }

        self.ctx.set_aliasing_threshold(None);
    }
}

trait Splat4thExt {
    fn splat_4th(self) -> Self;
}

impl<S: Simd> Splat4thExt for u8x32<S> {
    #[inline(always)]
    fn splat_4th(self) -> Self {
        [
            self[3], self[3], self[3], self[3], self[7], self[7], self[7], self[7], self[11],
            self[11], self[11], self[11], self[15], self[15], self[15], self[15], self[19],
            self[19], self[19], self[19], self[23], self[23], self[23], self[23], self[27],
            self[27], self[27], self[27], self[31], self[31], self[31], self[31],
        ]
        .simd_into(self.simd)
    }
}

fn premultiply_rgba(level: Level, data: &mut [u8]) {
    let simd_len = data.len() / 32 * 32;
    let (simd_data, tail) = data.split_at_mut(simd_len);

    #[inline(always)]
    fn premultiply_rgba_simd<S: Simd>(simd: S, data: &mut [u8]) {
        let alpha_lanes: mask8x32<S> = [
            0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0,
            -1, 0, 0, 0, -1,
        ]
        .simd_into(simd);
        for chunk in data.chunks_exact_mut(32) {
            let rgba = u8x32::from_slice(simd, chunk);
            let alphas = rgba.splat_4th();
            let premultiplied = (simd.widen_u8x32(rgba) * simd.widen_u8x32(alphas)).div_255();
            let premultiplied = simd.narrow_u16x32(premultiplied);
            alpha_lanes.select(rgba, premultiplied).store_slice(chunk);
        }
    }

    fearless_simd::dispatch!(level, simd => premultiply_rgba_simd(simd, simd_data));

    for pixel in tail.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        pixel[0] = div_255(u16::from(pixel[0]) * alpha) as u8;
        pixel[1] = div_255(u16::from(pixel[1]) * alpha) as u8;
        pixel[2] = div_255(u16::from(pixel[2]) * alpha) as u8;
    }
}

trait Div255Ext {
    fn div_255(self) -> Self;
}

impl<S: Simd> Div255Ext for u16x32<S> {
    #[inline(always)]
    fn div_255(self) -> Self {
        (self + Self::splat(self.simd, 255)) >> 8
    }
}

#[inline(always)]
const fn div_255(value: u16) -> u16 {
    (value + 255) >> 8
}
