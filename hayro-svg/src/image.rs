use crate::Id;
use crate::SvgRenderer;
use crate::mask::{ImageLuminanceMask, MaskKind};
use base64::Engine;
use hayro_interpret::{BlendMode, Device, FillRule, LumaData, Paint, PathDrawMode, RgbData};
use image::{DynamicImage, ImageBuffer, ImageFormat};
use kurbo::{Affine, Rect, Shape};
use std::io::Cursor;
use std::sync::Arc;

impl<'a> SvgRenderer<'a> {
    pub(crate) fn draw_rgba_image(
        &mut self,
        rgb_data: RgbData,
        transform: Affine,
        alpha: Option<LumaData>,
    ) {
        if let Some(alpha) = alpha {
            if alpha.interpolate == rgb_data.interpolate
                && alpha.width == rgb_data.width
                && alpha.height == rgb_data.height
            {
                let interleaved = rgb_data
                    .data
                    .chunks(3)
                    .zip(alpha.data)
                    .flat_map(|(rgb, a)| [rgb[0], rgb[1], rgb[2], a])
                    .collect::<Vec<u8>>();

                let image = DynamicImage::ImageRgba8(
                    ImageBuffer::from_raw(rgb_data.width, rgb_data.height, interleaved).unwrap(),
                );

                self.write_image(&image, rgb_data.interpolate, None, transform);
            } else {
                let image = DynamicImage::ImageRgb8(
                    ImageBuffer::from_raw(rgb_data.width, rgb_data.height, rgb_data.data.clone())
                        .unwrap(),
                );

                let alpha = {
                    let image = DynamicImage::ImageLuma8(
                        ImageBuffer::from_raw(alpha.width, alpha.height, alpha.data).unwrap(),
                    );

                    let transform = transform
                        * Affine::scale_non_uniform(
                            rgb_data.width as f64 / alpha.width as f64,
                            rgb_data.height as f64 / alpha.height as f64,
                        );

                    ImageLuminanceMask {
                        image,
                        transform,
                        interpolate: alpha.interpolate,
                    }
                };

                self.push_transparency_group_inner(
                    1.0,
                    Some(MaskKind::Image(Arc::new(alpha))),
                    BlendMode::Normal,
                );
                self.write_image(&image, rgb_data.interpolate, None, transform);
                self.pop_transparency_group();
            }
        } else {
            let image = DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(rgb_data.width, rgb_data.height, rgb_data.data.clone())
                    .unwrap(),
            );

            self.write_image(&image, rgb_data.interpolate, None, transform);
        };
    }

    pub(crate) fn draw_stencil_image(
        &mut self,
        stencil: LumaData,
        transform: Affine,
        paint: &Paint<'a>,
    ) {
        let interpolate = stencil.interpolate;

        match &paint {
            Paint::Color(c) => {
                let color = c.to_rgba().to_rgba8();
                let image = stencil
                    .data
                    .iter()
                    .flat_map(|d| if *d == 255 { color } else { [0, 0, 0, 0] })
                    .collect::<Vec<u8>>();

                let image = DynamicImage::ImageRgba8(
                    ImageBuffer::from_raw(stencil.width, stencil.height, image).unwrap(),
                );

                self.write_image(&image, interpolate, None, transform);
            }
            Paint::Pattern(_) => {
                let mask = {
                    let image = DynamicImage::ImageLuma8(
                        ImageBuffer::from_raw(stencil.width, stencil.height, stencil.data).unwrap(),
                    );

                    ImageLuminanceMask {
                        image,
                        transform,
                        interpolate,
                    }
                };

                self.push_transparency_group_inner(
                    1.0,
                    Some(MaskKind::Image(Arc::new(mask))),
                    BlendMode::Normal,
                );
                self.draw_path(
                    &Rect::new(0.0, 0.0, stencil.width as f64, stencil.height as f64).to_path(0.1),
                    transform,
                    paint,
                    &PathDrawMode::Fill(FillRule::NonZero),
                );
                self.pop_transparency_group();
            }
        };
    }

    pub(crate) fn write_image(
        &mut self,
        image: &DynamicImage,
        interpolate: bool,
        id: Option<Id>,
        transform: Affine,
    ) {
        let scaling = if interpolate { "smooth" } else { "pixelated" };

        let base64 = to_base64(image);

        self.xml.start_element("image");
        if let Some(id) = id {
            self.xml.write_attribute("id", &id);
        }
        self.write_transform(transform);
        self.xml.write_attribute("xlink:href", &base64);
        self.xml.write_attribute("width", &image.width());
        self.xml.write_attribute("height", &image.height());
        self.xml.write_attribute("preserveAspectRatio", "none");
        self.xml
            .write_attribute("style", &format_args!("image-rendering: {scaling}"));
        self.xml.end_element();
    }
}

pub(crate) fn to_base64(image: &DynamicImage) -> String {
    let mut png_buffer = Vec::new();
    let mut cursor = Cursor::new(&mut png_buffer);
    image.write_to(&mut cursor, ImageFormat::Png).unwrap();

    let mut url = "data:image/png;base64,".to_string();
    let data = base64::engine::general_purpose::STANDARD.encode(png_buffer);
    url.push_str(&data);

    url
}
