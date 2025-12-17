use image::{GrayImage, Luma};

fn main() {
    let base_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-inputs/serenity/");
    let filename = "bitmap-mmr.jbig2";

    let path = format!("{base_path}{filename}");
    let data = std::fs::read(&path).expect("Failed to read test file");

    println!("Decoding: {filename} ({} bytes)", data.len());

    let image = hayro_jbig2::decode(&data).expect("Failed to decode JBIG2");

    println!("Decoded: {}x{} image", image.width, image.height);

    // Convert to grayscale image (1-bit packed -> 8-bit grayscale).
    // In JBIG2: 1 = black, 0 = white.
    let mut gray = GrayImage::new(image.width, image.height);
    let stride = image.stride();

    for y in 0..image.height {
        for x in 0..image.width {
            let byte_idx = y as usize * stride + (x as usize / 8);
            let bit_idx = 7 - (x as usize % 8);
            let pixel = (image.data[byte_idx] >> bit_idx) & 1;
            // 1 = black (0), 0 = white (255)
            let luma = if pixel == 1 { 0 } else { 255 };
            gray.put_pixel(x, y, Luma([luma]));
        }
    }

    gray.save("out.png").expect("Failed to save PNG");
    println!("Saved: out.png");
}
