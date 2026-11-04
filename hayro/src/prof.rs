//! VeriPDF: coarse device-side timing counters (debug builds of the
//! renderer's hot entry points).

// VeriPDF: coarse device-side timing, printed via
// VERIPDF_RENDERER_DEBUG from the backend's per-tile debug line.
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) static T_PATH: AtomicU64 = AtomicU64::new(0);
pub(crate) static T_GLYPH: AtomicU64 = AtomicU64::new(0);
pub(crate) static T_IMAGE: AtomicU64 = AtomicU64::new(0);
pub(crate) static T_MASK: AtomicU64 = AtomicU64::new(0);
pub(crate) static T_PAINT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Scope<'a>(&'a AtomicU64, std::time::Instant);
impl<'a> Scope<'a> {
    pub(crate) fn start(counter: &'a AtomicU64) -> Self {
        Self(counter, std::time::Instant::now())
    }
}
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.0
            .fetch_add(self.1.elapsed().as_micros() as u64, Ordering::Relaxed);
    }
}

/// Microseconds spent in each device category so far on this thread-group.
pub fn snapshot() -> (u64, u64, u64, u64, u64) {
    (
        T_PATH.load(Ordering::Relaxed),
        T_GLYPH.load(Ordering::Relaxed),
        T_IMAGE.load(Ordering::Relaxed),
        T_MASK.load(Ordering::Relaxed),
        T_PAINT.load(Ordering::Relaxed),
    )
}
