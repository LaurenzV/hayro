//! Each test decodes a raw CCITT stream from `streams` and compares the result
//! against an image in `references`.

mod codes;
mod dimensions;
mod edges;
mod g3_1d;
mod g3_2d;
mod g4;
mod modes;
mod recovery;
mod settings;

use hayro_ccitt::{DecodeSettings, Decoder, DecoderContext, EncodingMode, decode};
use std::path::PathBuf;

const G3_1D: EncodingMode = EncodingMode::Group3_1D;
const G3_2D: EncodingMode = EncodingMode::Group3_2D { k: 2 };
const G4: EncodingMode = EncodingMode::Group4;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Rows of pixels, where `true` means white.
type Bitmap = Vec<Vec<bool>>;

fn load_reference(name: &str) -> Bitmap {
    let path = tests_dir().join("references").join(format!("{name}.png"));
    let image = image::open(&path)
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", path.display()))
        .to_luma8();

    image
        .rows()
        .map(|row| row.map(|p| p.0[0] > 127).collect())
        .collect()
}

#[derive(Default)]
struct Collector {
    rows: Bitmap,
    current: Vec<bool>,
}

impl Decoder for Collector {
    fn push_pixels(&mut self, white: bool, count: u32) {
        self.current
            .extend(std::iter::repeat_n(white, count as usize));
    }

    fn next_line(&mut self) {
        self.rows.push(std::mem::take(&mut self.current));
    }
}

/// Decode `stream` and check that it matches `reference` and that the whole
/// stream was consumed.
fn check(stream: &str, reference: &str, configure: impl FnOnce(&mut DecodeSettings)) {
    check_consumed(stream, reference, None, configure);
}

fn check_consumed(
    stream: &str,
    reference: &str,
    consumed: Option<usize>,
    configure: impl FnOnce(&mut DecodeSettings),
) {
    let path = tests_dir().join("streams").join(format!("{stream}.bin"));
    let data =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let reference = load_reference(reference);

    let mut settings = DecodeSettings {
        columns: reference[0].len() as u32,
        rows: reference.len() as u32,
        end_of_block: false,
        end_of_line: false,
        rows_are_byte_aligned: false,
        encoding: G4,
        invert_black: false,
    };
    configure(&mut settings);

    let mut collector = Collector::default();
    let mut ctx = DecoderContext::new(settings);
    let read = decode(&data, &mut collector, &mut ctx)
        .unwrap_or_else(|e| panic!("{stream}: decoding failed: {e}"));

    let expected_rows = if settings.rows == 0 {
        reference.len()
    } else {
        reference.len().min(settings.rows as usize)
    };
    assert_eq!(
        collector.rows.len(),
        expected_rows,
        "{stream}: wrong number of rows"
    );

    for (y, (actual, expected)) in collector.rows.iter().zip(&reference).enumerate() {
        assert_eq!(
            actual.len(),
            expected.len(),
            "{stream}: row {y} has the wrong length"
        );

        if let Some(x) = actual.iter().zip(expected).position(|(a, e)| a != e) {
            panic!("{stream}: first mismatch at row {y}, column {x}");
        }
    }

    assert_eq!(
        read,
        consumed.unwrap_or(data.len()),
        "{stream}: wrong number of bytes consumed"
    );
}
