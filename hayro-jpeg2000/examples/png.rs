use hayro_jpeg2000::read;
use image::{DynamicImage, ImageBuffer};

fn main() {
    let data = std::fs::read("hayro-jpeg2000/test.jp2").unwrap();

    match read(&data) {
        Ok(bitmap) => {
            let (width, height) = (bitmap.metadata.width, bitmap.metadata.height);

            let channels = bitmap
                .channels
                .into_iter()
                .map(|c| c.into_8bit())
                .collect::<Vec<_>>();

            let dynamic = match channels.len() {
                1 => DynamicImage::ImageLuma8(
                    ImageBuffer::from_raw(width, height, channels[0].clone()).unwrap(),
                ),
                _ => unimplemented!(),
            };

            dynamic.save("out.png").unwrap();

            // println!("Image Metadata:");
            // println!("  Width: {}", metadata.width);
            // println!("  Height: {}", metadata.height);
            // println!("  Components: {}", metadata.num_components);
            // println!("  Bits per component: {}", metadata.bits_per_component);
            // println!("  Has IP: {}", metadata.has_intellectual_property);
            //
            // if let Some(method) = metadata.colour_method {
            //     println!("  Colour method: {}", method);
            //     if let Some(enum_cs) = metadata.enumerated_colourspace {
            //         println!("  Enumerated colourspace: {}", enum_cs);
            //     }
            //     if let Some(ref profile) = metadata.icc_profile {
            //         println!("  ICC profile size: {} bytes", profile.len());
            //     }
            // }
        }
        Err(e) => {
            println!("Failed to read JP2 file: {}", e);
        }
    }
}
