use crate::OutlineError;

pub(crate) struct ArgumentsStack<'a> {
    pub data: &'a mut [f32],
    pub len: usize,
    pub max_len: usize,
}

impl<'a> ArgumentsStack<'a> {
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub(crate) fn push(&mut self, n: f32) -> Result<(), OutlineError> {
        if self.len == self.max_len {
            Err(OutlineError::ArgumentsStackLimitReached)
        } else {
            self.data[self.len] = n;
            self.len += 1;
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn at(&self, index: usize) -> f32 {
        self.data[index]
    }

    #[inline]
    pub(crate) fn pop(&mut self) -> f32 {
        debug_assert!(!self.is_empty());
        self.len -= 1;
        self.data[self.len]
    }

    pub(crate) fn dump(&self) -> String {
        format!("{:?}", &self.data[0..self.len])
    }

    #[inline]
    pub(crate) fn reverse(&mut self) {
        if self.is_empty() {
            return;
        }

        // Reverse only the actual data and not the whole stack.
        let (first, _) = self.data.split_at_mut(self.len);
        first.reverse();
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    pub(crate) fn exch(&mut self) {
        let len = self.len();
        debug_assert!(len > 1);
        self.data.swap(len - 1, len - 2);
    }
}

impl core::fmt::Debug for ArgumentsStack<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(&self.data[..self.len]).finish()
    }
}
