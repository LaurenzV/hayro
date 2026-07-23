//! Cooperative cancellation via [`Image::decode_with_stop`].

use core::sync::atomic::{AtomicUsize, Ordering};

use enough::{Stop, StopReason};
use hayro_jpeg2000::{DecodeError, DecodeSettings, DecoderContext, Image};

const SAMPLE: &[u8] = include_bytes!("../assets/stop-test.jp2");

/// A stop that fires after `n` successful polls.
struct StopAfter(AtomicUsize);

impl Stop for StopAfter {
    fn check(&self) -> Result<(), StopReason> {
        if self.0.load(Ordering::Relaxed) == 0 {
            Err(StopReason::Cancelled)
        } else {
            self.0.fetch_sub(1, Ordering::Relaxed);
            Ok(())
        }
    }
}

fn decode_with(stop: impl Stop) -> Result<(), DecodeError> {
    let image = Image::new(SAMPLE, &DecodeSettings::default()).unwrap();
    let mut ctx = DecoderContext::default();
    image.decode_with_stop(&mut ctx, &stop).map(|_| ())
}

#[test]
fn never_stopping_decodes_normally() {
    // A never-firing stop must not perturb a normal decode.
    let mut ctx = DecoderContext::default();
    Image::new(SAMPLE, &DecodeSettings::default())
        .unwrap()
        .decode(&mut ctx)
        .expect("plain decode");

    decode_with(StopAfter(AtomicUsize::new(usize::MAX))).expect("max-poll stop");
    decode_with(enough::Unstoppable).expect("unstoppable");
}

#[test]
fn stopping_returns_stopped() {
    // The sample decodes through at least a tile and a code block, so stopping
    // after 0 or 1 polls aborts with `Stopped`.
    for polls in [0, 1] {
        let err = decode_with(StopAfter(AtomicUsize::new(polls))).unwrap_err();
        assert!(matches!(err, DecodeError::Stopped(_)));
    }
}
