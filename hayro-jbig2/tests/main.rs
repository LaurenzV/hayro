//! Decodes the JBIG2 files in `streams` and compares them against `references`.

mod cancel;
mod composite;
mod file;
mod generic;
mod halftone;
mod huffman;
mod lenient;
mod malformed;
mod memory;
mod page;
mod refinement;
mod symbol;
mod text;

use std::path::PathBuf;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

type Bitmap = Vec<Vec<bool>>;

fn load_reference(name: &str) -> Bitmap {
    let path = tests_dir().join("references").join(format!("{name}.png"));
    let image = image::open(&path)
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", path.display()))
        .to_luma8();

    image
        .rows()
        .map(|row| row.map(|p| p.0[0] < 128).collect())
        .collect()
}

fn decode(page: &hayro_jbig2::Page<'_>) -> hayro_jbig2::Result<Bitmap> {
    let bitmap = page.decode()?;
    let width = bitmap.width() as usize;

    Ok(bitmap
        .rows()
        .map(|row| {
            (0..width)
                .map(|x| row[x / 32] & (0x8000_0000 >> (x % 32)) != 0)
                .collect()
        })
        .collect())
}

fn check(stream: &str, reference: &str) {
    compare(stream, load_reference(reference), true);
}

fn check_with_memory_limit(stream: &str, reference: &str, memory_limit: usize) {
    let settings = hayro_jbig2::DecodeSettings {
        memory_limit: Some(memory_limit),
        ..Default::default()
    };
    compare_with(stream, load_reference(reference), &settings);
}

fn check_memory_limit_exceeded(stream: &str, memory_limit: usize) {
    let data = read_stream(stream);
    let settings = hayro_jbig2::DecodeSettings {
        memory_limit: Some(memory_limit),
        ..Default::default()
    };
    let file = hayro_jbig2::File::new(&data, &settings)
        .unwrap_or_else(|e| panic!("{stream}: parsing failed: {e}"));
    let page = file.page(0).unwrap_or_else(|| panic!("{stream}: no pages"));

    assert_eq!(
        page.decode().err(),
        Some(hayro_jbig2::DecodeError::MemoryLimitExceeded),
        "{stream}: memory limit not exceeded"
    );
}

fn check_lenient(stream: &str, reference: &str) {
    compare(stream, load_reference(reference), false);
}

fn check_pages(stream: &str, references: &[&str]) {
    compare_pages(stream, references, true);
}

fn check_pages_lenient(stream: &str, references: &[&str]) {
    compare_pages(stream, references, false);
}

fn compare_pages(stream: &str, references: &[&str], strict: bool) {
    let data = read_stream(stream);
    let file = hayro_jbig2::File::new(
        &data,
        &hayro_jbig2::DecodeSettings {
            strict,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("{stream}: parsing failed: {e}"));
    assert_eq!(
        file.page_count(),
        references.len(),
        "{stream}: wrong number of pages"
    );

    let mut previous_number = 0;
    for (page, reference) in file.pages().zip(references) {
        assert!(
            page.number() > previous_number,
            "{stream}: page {} after page {previous_number}",
            page.number()
        );
        previous_number = page.number();
        compare_page(stream, &page, load_reference(reference));
    }
}

fn check_crop(stream: &str, reference: &str, (x, y, width, height): (usize, usize, usize, usize)) {
    let reference = load_reference(reference)[y..y + height]
        .iter()
        .map(|row| row[x..x + width].to_vec())
        .collect();
    compare(stream, reference, true);
}

fn compare(stream: &str, reference: Bitmap, strict: bool) {
    let settings = hayro_jbig2::DecodeSettings {
        strict,
        ..Default::default()
    };
    compare_with(stream, reference, &settings);
}

fn compare_with(stream: &str, reference: Bitmap, settings: &hayro_jbig2::DecodeSettings) {
    let data = read_stream(stream);
    let file = hayro_jbig2::File::new(&data, settings)
        .unwrap_or_else(|e| panic!("{stream}: parsing failed: {e}"));
    let page = file.page(0).unwrap_or_else(|| panic!("{stream}: no pages"));
    compare_page(stream, &page, reference);
}

fn read_stream(stream: &str) -> Vec<u8> {
    let path = tests_dir().join("streams").join(format!("{stream}.jbig2"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn compare_page(stream: &str, page: &hayro_jbig2::Page<'_>, reference: Bitmap) {
    assert_eq!(
        (page.width() as usize, page.height() as usize),
        (reference[0].len(), reference.len()),
        "{stream}: wrong page size"
    );

    let rows = decode(page).unwrap_or_else(|e| panic!("{stream}: decoding failed: {e}"));

    assert_eq!(
        rows.len(),
        reference.len(),
        "{stream}: wrong number of rows"
    );

    for (y, (actual, expected)) in rows.iter().zip(&reference).enumerate() {
        if let Some(x) = actual.iter().zip(expected).position(|(a, e)| a != e) {
            panic!("{stream}: first mismatch at row {y}, column {x}");
        }
    }
}

fn check_malformed(stream: &str) {
    let data = read_stream(stream);

    if let Ok(file) = hayro_jbig2::File::new(&data, &hayro_jbig2::DecodeSettings::default()) {
        for page in file.pages() {
            let _ = decode(&page);
        }
    }
}
