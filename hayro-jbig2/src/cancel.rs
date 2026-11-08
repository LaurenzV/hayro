use crate::bitmap::WORD_BITS;
use crate::error::Cancelled;

/// A check for cancelling decoding.
pub trait CancelCheck: Send + Sync {
    /// Returns whether decoding should be cancelled.
    fn is_cancelled(&self) -> bool;
}

impl<F: Fn() -> bool + Send + Sync> CancelCheck for F {
    fn is_cancelled(&self) -> bool {
        self()
    }
}

/// A shared [`CancelCheck`] that can be stored in the decode settings.
#[derive(Clone)]
pub struct CancelHandle(hayro_ccitt::CancelHandle);

impl CancelHandle {
    /// Create a new handle from a check.
    pub fn new(check: impl CancelCheck + 'static) -> Self {
        Self(hayro_ccitt::CancelHandle::new(move || check.is_cancelled()))
    }

    /// Returns whether decoding should be cancelled.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl core::fmt::Debug for CancelHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CancelHandle").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Cancel<'a>(pub(crate) Option<&'a CancelHandle>);

impl Cancel<'_> {
    #[inline(always)]
    pub(crate) fn check(self) -> Result<(), Cancelled> {
        if self.0.is_some_and(|c| c.is_cancelled()) {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn ccitt(self) -> Option<hayro_ccitt::CancelHandle> {
        self.0.map(|handle| handle.0.clone())
    }
}

// The amount of work between two checks, in units of about one decoded pixel.
const CHECK_INTERVAL: u32 = 1 << 16;

// Copying or combining a word of pixels costs about as much as decoding four pixels.
pub(crate) const WORD_WORK: u32 = 4;

// Filling a byte costs about 1/64 of decoding a pixel.
pub(crate) const FILL_CHUNK: usize = 64 * CHECK_INTERVAL as usize;

pub(crate) struct WorkCounter(u64);

impl WorkCounter {
    pub(crate) fn new() -> Self {
        Self(0)
    }

    #[inline(always)]
    pub(crate) fn add(&mut self, work: u64, cancel: Cancel<'_>) -> Result<(), Cancelled> {
        self.0 += work;

        if self.0 >= u64::from(CHECK_INTERVAL) {
            self.0 = 0;
            cancel.check()?;
        }

        Ok(())
    }
}

pub(crate) fn copy_work(width: u32) -> u32 {
    width.div_ceil(WORD_BITS) * WORD_WORK
}

#[inline]
pub(crate) fn row_mask(row_work: u32) -> u32 {
    // Rows also cost something apart from their pixels, which matters for
    // very narrow bitmaps.
    let rows = (CHECK_INTERVAL / row_work.max(1)).clamp(1, 1024);
    (1 << rows.ilog2()) - 1
}
