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
const G3_2D: EncodingMode = EncodingMode::Group3_2D;
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

struct Collector {
    columns: u32,
    rows: Bitmap,
}

impl Collector {
    fn new(columns: u32) -> Self {
        Self {
            columns,
            rows: Vec::new(),
        }
    }
}

impl Decoder for Collector {
    fn push_row(&mut self, changes: &[u32]) {
        assert!(changes.windows(2).all(|w| w[0] < w[1]));
        assert!(changes.iter().all(|&c| c < self.columns));

        let row = (0..self.columns)
            .map(|x| changes.partition_point(|&c| c <= x) % 2 == 0)
            .collect();
        self.rows.push(row);
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

    let mut settings = DecodeSettings::new(reference[0].len() as u32, reference.len() as u32);
    settings.end_of_block = false;
    settings.encoding = G4;
    configure(&mut settings);

    let mut collector = Collector::new(settings.columns);
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

/// Decode `stream`, which is malformed, and check that it doesn't panic.
fn check_malformed(
    stream: &str,
    columns: u32,
    rows: u32,
    configure: impl FnOnce(&mut DecodeSettings),
) {
    let path = tests_dir().join("streams").join(format!("{stream}.bin"));
    let data =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

    let mut settings = DecodeSettings::new(columns, rows);
    configure(&mut settings);

    let mut ctx = DecoderContext::new(settings);
    let _ = decode(&data, &mut Collector::new(columns), &mut ctx);
}
