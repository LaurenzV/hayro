use crate::sync::Arc;

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

/// A shared [`CancelCheck`].
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
