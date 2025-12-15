fn main() {
    let base_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-inputs/serenity/");
    let filename = "bitmap-mmr.jbig2";

    let path = format!("{base_path}{filename}");
    let data = std::fs::read(&path).expect("Failed to read test file");

    println!("================================================================================");
    println!("File: {filename}");
    println!("Size: {} bytes", data.len());
    println!("================================================================================");

    hayro_jbig2::debug_parse_file(&data);
}
