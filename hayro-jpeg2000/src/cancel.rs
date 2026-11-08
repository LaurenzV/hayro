use crate::error::{Cancelled, Result};
use crate::sync::Arc;
use alloc::vec::Vec;

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
pub struct CancelHandle(Arc<dyn CancelCheck>);

impl CancelHandle {
    /// Create a new handle from a check.
    pub fn new(check: impl CancelCheck + 'static) -> Self {
        Self(Arc::new(check))
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

pub(crate) const CHUNK: usize = 1 << 16;

#[derive(Clone, Copy)]
pub(crate) struct Cancel<'a>(pub(crate) Option<&'a CancelHandle>);

impl Cancel<'_> {
    #[inline(always)]
    pub(crate) fn is_cancelled(self) -> bool {
        self.0.is_some_and(|c| c.is_cancelled())
    }

    #[inline(always)]
    pub(crate) fn check(self) -> core::result::Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn resize(self, buf: &mut Vec<f32>, len: usize) -> Result<()> {
        buf.truncate(len);
        buf.reserve(len - buf.len());

        while buf.len() < len {
            self.check()?;

            let next = len.min(buf.len() + CHUNK);
            buf.resize(next, 0.0);
        }

        Ok(())
    }
}
