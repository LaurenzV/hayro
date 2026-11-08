//! Decoding with a cancellation check.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hayro_jbig2::{CancelHandle, Cancelled, DecodeError, DecodeSettings, File};

use crate::read_stream;

#[test]
fn cancelled() {
    let data = read_stream("generic_template0");
    let settings = DecodeSettings {
        cancel: Some(CancelHandle::new(|| true)),
        ..Default::default()
    };
    let file = File::new(&data, &settings).unwrap();

    assert_eq!(
        file.page(0).unwrap().decode().err(),
        Some(DecodeError::Cancelled)
    );
}

#[test]
fn data_u8() {
    let data = read_stream("generic_template0");
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let settings = DecodeSettings {
        cancel: Some(CancelHandle::new(move || flag.load(Ordering::Relaxed))),
        ..Default::default()
    };
    let file = File::new(&data, &settings).unwrap();
    let bitmap = file.page(0).unwrap().decode().unwrap();

    cancelled.store(true, Ordering::Relaxed);
    let mut buf = vec![0; bitmap.width() as usize * bitmap.height() as usize];

    assert_eq!(bitmap.data_u8(), Err(Cancelled));
    assert_eq!(bitmap.store_u8_into(&mut buf), Err(Cancelled));
}
