//! Each test decodes a JBIG2 file from `streams` and compares the result
//! against an image in `references`.

mod composite;
mod file;
mod generic;
mod halftone;
mod huffman;
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

/// Decode `stream` and check that it matches `reference`.
fn check(stream: &str, reference: &str) {
    compare(stream, load_reference(reference));
}

/// Decode `stream` and check that it matches the given part (x, y, width,
/// height) of `reference`.
fn check_crop(stream: &str, reference: &str, (x, y, width, height): (usize, usize, usize, usize)) {
    let reference = load_reference(reference)[y..y + height]
        .iter()
        .map(|row| row[x..x + width].to_vec())
        .collect();
    compare(stream, reference);
}

fn compare(stream: &str, reference: Bitmap) {
    let path = tests_dir().join("streams").join(format!("{stream}.jbig2"));
    let data =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

    let image =
        hayro_jbig2::Image::new(&data).unwrap_or_else(|e| panic!("{stream}: parsing failed: {e}"));
    let mut collector = Collector::default();
    image
        .decode(&mut collector)
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
