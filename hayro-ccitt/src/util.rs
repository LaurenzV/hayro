use crate::{DecodeError, Result};
use alloc::vec::Vec;

pub(crate) trait VecExt<T> {
    fn try_push(&mut self, value: T) -> Result<()>;
}

impl<T> VecExt<T> for Vec<T> {
    #[inline(always)]
    fn try_push(&mut self, value: T) -> Result<()> {
        if self.len() == self.capacity() {
            self.try_reserve(1).map_err(|_| DecodeError::OutOfMemory)?;
        }

        self.push(value);

        Ok(())
    }
}
