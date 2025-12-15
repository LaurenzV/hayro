fn main() {
    let manifest: Vec<String> = serde_json::from_str(
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/manifest_serenity.json"))
    ).expect("Failed to parse manifest");

    let base_path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-inputs/serenity/");

    for filename in &manifest {
        let path = format!("{base_path}{filename}");
        let data = std::fs::read(&path).expect("Failed to read test file");

        println!("================================================================================");
        println!("File: {filename}");
        println!("Size: {} bytes", data.len());
        println!("================================================================================");

        hayro_jbig2::debug_parse_file(&data);
        println!();
    }

    println!("Parsed {} files.", manifest.len());
}
