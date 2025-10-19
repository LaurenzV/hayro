use hayro_jpeg2000::read;

fn main() {
    let data = std::fs::read("indexed-small.jp2").unwrap();

    match read(&data) {
        Some(metadata) => {
            println!("Image Metadata:");
            println!("  Width: {}", metadata.width);
            println!("  Height: {}", metadata.height);
            println!("  Components: {}", metadata.num_components);
            println!("  Bits per component: {}", metadata.bits_per_component);
            println!("  Compression type: {}", metadata.compression_type);
            println!("  Colourspace unknown: {}", metadata.colourspace_unknown);
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
        None => {
            println!("Failed to read JP2 file");
        }
    }
}
