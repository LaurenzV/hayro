//! A basic example demonstrating how to convert a JBIG2 image into a PNG image.

fn main() {
    // let base_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-inputs/serenity/");
    // let filename = "bitmap-symbol-refine.jbig2";

    let path = "/Users/lstampfl/Programming/hayro/test.jb2";
    let data = std::fs::read(path).expect("Failed to read test file");

    // println!("Decoding: {filename} ({} bytes)", data.len());

    // let gray = jbig2::decode_to_image(&data).unwrap();
    for _ in 0..20 {
        // let _ = jbig2::decode(&data).unwrap();
        let _ = hayro_jbig2::decode(&data).expect("Failed to decode JBIG2");
    }

    // gray.save("out.png").expect("Failed to save PNG");
    println!("Saved: out.png");
}
