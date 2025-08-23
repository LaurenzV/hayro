use crate::Id;
use crate::renderer::SvgRenderer;
use base64::Engine;
use image::{DynamicImage, ImageFormat};
use kurbo::Affine;
use std::io::Cursor;

impl SvgRenderer<'_> {
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
