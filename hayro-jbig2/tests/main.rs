//! Each test decodes a JBIG2 file from `streams` and compares the result
//! against an image in `references`.

mod composite;
mod file;
mod generic;
mod halftone;
mod huffman;
mod lenient;
mod malformed;
mod page;
mod refinement;
mod symbol;
mod text;

use std::path::PathBuf;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Rows of pixels, where `true` means black.
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

#[derive(Default)]
struct Collector {
    rows: Bitmap,
    current: Vec<bool>,
}

impl hayro_jbig2::Decoder for Collector {
    fn push_pixel(&mut self, black: bool) {
        self.current.push(black);
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        self.current
            .extend(std::iter::repeat_n(black, chunk_count as usize * 8));
    }

    fn next_line(&mut self) {
        self.rows.push(std::mem::take(&mut self.current));
    }
}

/// Decode `stream` in strict mode and check that it matches `reference`.
fn check(stream: &str, reference: &str) {
    compare(stream, load_reference(reference), true);
}

/// Decode `stream`, which is invalid, in lenient mode and check that it
/// matches `reference`.
fn check_lenient(stream: &str, reference: &str) {
    compare(stream, load_reference(reference), false);
}

/// Decode all pages of `stream` in strict mode and check that they match
/// `references`.
fn check_pages(stream: &str, references: &[&str]) {
    compare_pages(stream, references, true);
}

fn compare_pages(stream: &str, references: &[&str], strict: bool) {
    let data = read_stream(stream);
    let file = hayro_jbig2::File::new(&data, &hayro_jbig2::DecodeSettings { strict })
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

/// Decode `stream` in strict mode and check that it matches the given part
/// (x, y, width, height) of `reference`.
fn check_crop(stream: &str, reference: &str, (x, y, width, height): (usize, usize, usize, usize)) {
    let reference = load_reference(reference)[y..y + height]
        .iter()
        .map(|row| row[x..x + width].to_vec())
        .collect();
    compare(stream, reference, true);
}

fn compare(stream: &str, reference: Bitmap, strict: bool) {
    let data = read_stream(stream);
    let file = hayro_jbig2::File::new(&data, &hayro_jbig2::DecodeSettings { strict })
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

    let mut collector = Collector::default();
    page.decode(&mut collector)
        .unwrap_or_else(|e| panic!("{stream}: decoding failed: {e}"));

    assert_eq!(
        collector.rows.len(),
        reference.len(),
        "{stream}: wrong number of rows"
    );

    for (y, (actual, expected)) in collector.rows.iter().zip(&reference).enumerate() {
        // Rows may be padded to whole chunks of 8 pixels.
        assert!(
            actual.len() >= expected.len(),
            "{stream}: row {y} is too short"
        );

        if let Some(x) = actual.iter().zip(expected).position(|(a, e)| a != e) {
            panic!("{stream}: first mismatch at row {y}, column {x}");
        }
    }
}

/// Decode `stream`, which is malformed, and check that it doesn't panic.
fn check_malformed(stream: &str) {
    let data = read_stream(stream);

    if let Ok(file) = hayro_jbig2::File::new(&data, &hayro_jbig2::DecodeSettings::default()) {
        for page in file.pages() {
            let _ = page.decode(&mut Collector::default());
        }
    }
}
