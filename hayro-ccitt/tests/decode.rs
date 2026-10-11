//! Each test decodes a raw CCITT stream from `streams` and compares the result
//! against an image in `references`.

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

// Group 4 stream structure.

#[test]
fn g4() {
    check("g4", "base", |_| {});
}

#[test]
fn g4_eofb() {
    check("g4_eofb", "base", |s| s.end_of_block = true);
}

#[test]
fn g4_eofb_pad_bits() {
    check_consumed("g4_eofb_pad_bits", "base", Some(1086), |s| {
        s.end_of_block = true;
    });
}

#[test]
fn g4_byte_align() {
    check("g4_byte_align", "base", |s| s.rows_are_byte_aligned = true);
}

#[test]
fn g4_byte_align_eofb() {
    check("g4_byte_align_eofb", "base", |s| {
        s.rows_are_byte_aligned = true;
        s.end_of_block = true;
    });
}

// Group 3 (1D) stream structure.

#[test]
fn g3_1d() {
    check("g3_1d", "base", |s| s.encoding = G3_1D);
}

#[test]
fn g3_1d_eol() {
    check("g3_1d_eol", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
    });
}

#[test]
fn g3_1d_eol_fill() {
    check("g3_1d_eol_fill", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
    });
}

#[test]
fn g3_1d_rtc() {
    check("g3_1d_rtc", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.end_of_block = true;
    });
}

#[test]
fn g3_1d_byte_align() {
    check("g3_1d_byte_align", "base", |s| {
        s.encoding = G3_1D;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn g3_1d_byte_align_eol() {
    check("g3_1d_byte_align_eol", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn g3_1d_byte_align_rtc() {
    check("g3_1d_byte_align_rtc", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.end_of_block = true;
        s.rows_are_byte_aligned = true;
    });
}

// Group 3 (2D) stream structure.

#[test]
fn g3_2d() {
    check("g3_2d", "base", |s| s.encoding = G3_2D);
}

#[test]
fn g3_2d_eol() {
    check("g3_2d_eol", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
    });
}

#[test]
fn g3_2d_eol_fill() {
    check("g3_2d_eol_fill", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
    });
}

#[test]
fn g3_2d_byte_align() {
    check("g3_2d_byte_align", "base", |s| {
        s.encoding = G3_2D;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn g3_2d_byte_align_eol() {
    check("g3_2d_byte_align_eol", "base", |s| {
        s.encoding = G3_2D;
        s.end_of_line = true;
        s.rows_are_byte_aligned = true;
    });
}

#[test]
fn g3_2d_all_1d() {
    check("g3_2d_all_1d", "base", |s| s.encoding = G3_2D);
}

#[test]
fn g3_2d_irregular_tags() {
    check("g3_2d_irregular_tags", "base", |s| {
        s.encoding = EncodingMode::Group3_2D { k: 4 };
    });
}

// Code tables.

#[test]
fn codes_g3_1d() {
    check("codes_g3_1d", "wedge", |s| s.encoding = G3_1D);
}

#[test]
fn codes_g3_2d() {
    check("codes_g3_2d", "wedge", |s| s.encoding = G3_2D);
}

#[test]
fn codes_g4() {
    check("codes_g4", "wedge", |_| {});
}

// Two-dimensional coding modes.

#[test]
fn mode_pass_g4() {
    check("mode_pass_g4", "mode_pass", |_| {});
}

#[test]
fn mode_pass_g3_2d() {
    check("mode_pass_g3_2d", "mode_pass", |s| s.encoding = G3_2D);
}

#[test]
fn mode_vertical_g4() {
    check("mode_vertical_g4", "mode_vertical", |_| {});
}

#[test]
fn mode_vertical_g3_2d() {
    check("mode_vertical_g3_2d", "mode_vertical", |s| {
        s.encoding = G3_2D;
    });
}

#[test]
fn mode_horizontal_g4() {
    check("mode_horizontal_g4", "mode_horizontal", |_| {});
}

#[test]
fn mode_horizontal_g3_2d() {
    check("mode_horizontal_g3_2d", "mode_horizontal", |s| {
        s.encoding = G3_2D;
    });
}

// Changing elements at the start and end of lines.

#[test]
fn edges_g4() {
    check("edges_g4", "edges", |_| {});
}

#[test]
fn edges_g3_1d() {
    check("edges_g3_1d", "edges", |s| s.encoding = G3_1D);
}

#[test]
fn edges_g3_2d() {
    check("edges_g3_2d", "edges", |s| s.encoding = G3_2D);
}

// Image dimensions.

#[test]
fn width_1_g4() {
    check("width_1_g4", "width_1", |_| {});
}

#[test]
fn width_1_g3_1d() {
    check("width_1_g3_1d", "width_1", |s| s.encoding = G3_1D);
}

#[test]
fn width_1_g3_2d() {
    check("width_1_g3_2d", "width_1", |s| s.encoding = G3_2D);
}

#[test]
fn height_1_g4() {
    check("height_1_g4", "height_1", |_| {});
}

#[test]
fn height_1_g3_1d() {
    check("height_1_g3_1d", "height_1", |s| s.encoding = G3_1D);
}

#[test]
fn height_1_g3_2d() {
    check("height_1_g3_2d", "height_1", |s| s.encoding = G3_2D);
}

// Decode settings.

#[test]
fn black_is_1() {
    check("black_is_1", "base", |s| s.invert_black = true);
}

#[test]
fn rows_fewer_than_data() {
    check_consumed("g4", "base", Some(127), |s| s.rows = 48);
}

#[test]
fn eofb_before_rows() {
    check("g4_eofb", "base", |s| {
        s.end_of_block = true;
        s.rows = 200;
    });
}

#[test]
fn rows_unknown_rtc() {
    check("g3_1d_rtc", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.end_of_block = true;
        s.rows = 0;
    });
}

#[test]
fn end_of_block_without_eofb() {
    check("g4", "base", |s| s.end_of_block = true);
}

#[test]
fn end_of_block_without_rtc() {
    check("g3_1d_eol", "base", |s| {
        s.encoding = G3_1D;
        s.end_of_line = true;
        s.end_of_block = true;
    });
}
