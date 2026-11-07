//! Decoding with a cancellation check.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use hayro_jbig2::{CancelHandle, Cancelled, DecodeError, DecodeSettings, File};

use crate::{compare_page, load_reference, read_stream};

const STREAMS: &[&str] = &[
    "composite_or_xor_replace",
    "generic_mmr",
    "generic_template0",
    "generic_template0_ext_custom_at",
    "halftone",
    "halftone_10_bitplanes_mmr",
    "halftone_rotated_grid_skip",
    "huffman_text_refine",
    "huffman_uncompressed_symbols",
    "refine_page",
    "refine_template0",
    "symbol_aggregate_then_refine",
    "symbol_global",
    "symbol_huffman_aggregate_then_refine",
    "text_many_instances",
    "text_refine",
];

fn settings(check: impl Fn() -> bool + Send + Sync + 'static) -> DecodeSettings {
    DecodeSettings {
        cancel: Some(CancelHandle::new(check)),
        ..Default::default()
    }
}

fn decode_with(data: &[u8], settings: &DecodeSettings) -> Result<(), DecodeError> {
    let file = File::new(data, settings).unwrap();
    file.page(0).unwrap().decode().map(|_| ())
}

#[test]
fn every_check() {
    for stream in STREAMS {
        let data = read_stream(stream);

        let checks = Arc::new(AtomicUsize::new(0));
        let counter = checks.clone();
        let counting = settings(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            false
        });
        decode_with(&data, &counting).unwrap_or_else(|e| panic!("{stream}: {e}"));
        let checks = checks.load(Ordering::Relaxed);

        for cancel_at in 0..checks {
            let count = AtomicUsize::new(0);
            let cancelling = settings(move || count.fetch_add(1, Ordering::Relaxed) == cancel_at);

            assert_eq!(
                decode_with(&data, &cancelling),
                Err(DecodeError::Cancelled),
                "{stream}: not cancelled at check {cancel_at} of {checks}"
            );
        }
    }
}

#[test]
fn globals_after_cancel() {
    let data = read_stream("symbol_global");
    let cancelled = Arc::new(AtomicBool::new(true));
    let flag = cancelled.clone();
    let settings = settings(move || flag.load(Ordering::Relaxed));
    let file = File::new(&data, &settings).unwrap();
    let page = file.page(0).unwrap();

    assert_eq!(page.decode().err(), Some(DecodeError::Cancelled));

    cancelled.store(false, Ordering::Relaxed);
    compare_page("symbol_global", &page, load_reference("honeycomb"));
}

#[test]
fn data_u8() {
    let data = read_stream("generic_template0");
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let settings = settings(move || flag.load(Ordering::Relaxed));
    let file = File::new(&data, &settings).unwrap();
    let bitmap = file.page(0).unwrap().decode().unwrap();

    cancelled.store(true, Ordering::Relaxed);
    let mut buf = vec![0; bitmap.width() as usize * bitmap.height() as usize];

    assert_eq!(bitmap.data_u8(), Err(Cancelled));
    assert_eq!(bitmap.store_u8_into(&mut buf), Err(Cancelled));
}
