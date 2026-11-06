use crate::error::{Cancelled, Result};
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

pub(crate) const CHUNK: usize = 1 << 16;

#[derive(Clone, Copy)]
pub(crate) struct Cancel<'a>(pub(crate) Option<&'a dyn CancelCheck>);

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
