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
        let image = hayro_jbig2::Image::new(&data).expect("Failed to parse JBIG2");
        struct NullDecoder;
        impl hayro_jbig2::Decoder for NullDecoder {
            fn push_pixel(&mut self, _black: bool) {}
            fn push_pixel_chunk(&mut self, _black: bool, _chunk_count: u32) {}
            fn next_line(&mut self) {}
        }
        let _ = image.decode(&mut NullDecoder);
    }

    // gray.save("out.png").expect("Failed to save PNG");
    println!("Saved: out.png");
}
