fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-inputs/serenity/bitmap-mmr.jbig2");
    let data = std::fs::read(path).expect("Failed to read test file");

    println!("Parsing JBIG2 file: {path}");
    println!("File size: {} bytes\n", data.len());

    hayro_jbig2::debug_parse_file(&data);
}
