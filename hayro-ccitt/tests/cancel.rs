use hayro_ccitt::{CancelHandle, DecodeError, DecodeSettings, decode};

use crate::{Collector, G4, load_reference, tests_dir};

#[test]
fn cancelled() {
    let data = std::fs::read(tests_dir().join("streams").join("g4.bin")).unwrap();
    let reference = load_reference("base");
    let settings = DecodeSettings {
        columns: reference[0].len() as u32,
        rows: reference.len() as u32,
        end_of_block: false,
        end_of_line: false,
        rows_are_byte_aligned: false,
        encoding: G4,
        strict: false,
        cancel: Some(CancelHandle::new(|| true)),
    };
    let mut collector = Collector::new(settings.columns);

    assert_eq!(
        decode(&data, &mut collector, &settings),
        Err(DecodeError::Cancelled)
    );
}
