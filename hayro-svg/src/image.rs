use crate::Id;
use crate::SvgRenderer;
use base64::Engine;
use hayro_interpret::{LumaData, Paint, RgbData};
use image::{DynamicImage, ImageBuffer, ImageFormat};
use kurbo::Affine;
use std::io::Cursor;

impl SvgRenderer<'_> {
    pub(crate) fn draw_rgba_image(
        &mut self,
        image: RgbData,
        transform: Affine,
        alpha: Option<LumaData>,
    ) {
        // TODO: Cache images
        let interpolate = image.interpolate;

        let image = if let Some(alpha) = alpha {
            if alpha.interpolate == image.interpolate
                && alpha.width == image.width
                && alpha.height == image.height
            {
                let interleaved = image
                    .data
                    .chunks(3)
                    .zip(alpha.data)
                    .flat_map(|(rgb, a)| [rgb[0], rgb[1], rgb[2], a])
                    .collect::<Vec<u8>>();

                DynamicImage::ImageRgba8(
                    ImageBuffer::from_raw(image.width, image.height, interleaved).unwrap(),
                )
            } else {
                unimplemented!();
            }
        } else {
            DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(image.width, image.height, image.data.clone()).unwrap(),
            )
        };

        self.write_image(&image, interpolate, None, transform);
    }

    pub(crate) fn draw_stencil_image(
        &mut self,
        stencil: LumaData,
        transform: Affine,
        paint: &Paint,
    ) {
        let interpolate = stencil.interpolate;

        let image = match &paint {
            Paint::Color(c) => {
                let color = c.to_rgba().to_rgba8();
                let image = stencil
                    .data
                    .iter()
                    .flat_map(|d| if *d == 255 { color } else { [0, 0, 0, 0] })
                    .collect::<Vec<u8>>();

                DynamicImage::ImageRgba8(
                    ImageBuffer::from_raw(stencil.width, stencil.height, image).unwrap(),
                )
            }
            Paint::Pattern(_) => {
                unreachable!();
            }
        };

        self.write_image(&image, interpolate, None, transform);
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
