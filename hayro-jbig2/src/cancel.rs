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
pub(crate) struct Cancel<'a>(Option<&'a CancelHandle>);

impl<'a> Cancel<'a> {
    pub(crate) fn new(handle: Option<&'a CancelHandle>) -> Self {
        Self(handle)
    }

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

pub(crate) struct WorkCounter {
    work: u64,
    limit: u64,
}

impl WorkCounter {
    pub(crate) fn new(limit: u64) -> Self {
        Self { work: 0, limit }
    }

    #[inline(always)]
    pub(crate) fn add(&mut self, work: u64, cancel: Cancel<'_>) -> Result<(), Cancelled> {
        self.work += work;

        if self.work >= self.limit {
            self.work = 0;
            cancel.check()?;
        }

        Ok(())
    }
}

#[inline]
pub(crate) fn row_mask(width: u32) -> u32 {
    let rows = ((1 << 16) / width.max(1)).clamp(1, 1024);
    (1 << rows.ilog2()) - 1
}
