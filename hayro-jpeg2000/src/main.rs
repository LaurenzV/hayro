use hayro_jpeg2000::read;

fn main() {
    let data = std::fs::read("/Users/lstampfl/Programming/GitHub/hayro/hayro-jpeg2000/test.jp2").unwrap();

    match read(&data) {
        Ok(metadata) => {
            println!("Image Metadata:");
            println!("  Width: {}", metadata.width);
            println!("  Height: {}", metadata.height);
            println!("  Components: {}", metadata.num_components);
            println!("  Bits per component: {}", metadata.bits_per_component);
            println!("  Has IP: {}", metadata.has_intellectual_property);

            if let Some(method) = metadata.colour_method {
                println!("  Colour method: {}", method);
                if let Some(enum_cs) = metadata.enumerated_colourspace {
                    println!("  Enumerated colourspace: {}", enum_cs);
                }
                if let Some(ref profile) = metadata.icc_profile {
                    println!("  ICC profile size: {} bytes", profile.len());
                }
            }
        }
        Err(e) => {
            println!("Failed to read JP2 file: {}", e);
        }
    }
}
