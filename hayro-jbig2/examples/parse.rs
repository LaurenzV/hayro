use image::{GrayImage, Luma};

fn main() {
    let base_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-inputs/serenity/");
    let filename = "bitmap-refine-page-subrect.jbig2";

    let path = format!("{base_path}{filename}");
    let data = std::fs::read(&path).expect("Failed to read test file");

    println!("Decoding: {filename} ({} bytes)", data.len());

    let image = hayro_jbig2::decode(&data).expect("Failed to decode JBIG2");

    println!("Decoded: {}x{} image", image.width, image.height);

    // Convert to grayscale image.
    // In JBIG2: true = black, false = white.
    let mut gray = GrayImage::new(image.width, image.height);

    for y in 0..image.height {
        for x in 0..image.width {
            let pixel = image.data[(y * image.width + x) as usize];
            // true = black (0), false = white (255)
            let luma = if pixel { 0 } else { 255 };
            gray.put_pixel(x, y, Luma([luma]));
        }
    }

    gray.save("out.png").expect("Failed to save PNG");
    println!("Saved: out.png");
}
