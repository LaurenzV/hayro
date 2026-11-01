//! A byte reader.

use crate::budget::{ParseBudget, WORK_WINDOW};
use core::ops::Range;

/// A reader for reading bytes and PDF objects.
#[derive(Clone)]
pub struct Reader<'a> {
    /// The underlying data of the reader.
    pub data: &'a [u8],
    /// The current byte-offset.
    pub offset: usize,
    /// The offset up to which reading was charged to `budget`, or
    /// `usize::MAX` without a budget.
    charged_to: usize,
    budget: Option<&'a dyn ParseBudget>,
}

impl core::fmt::Debug for Reader<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Reader")
            .field("data", &self.data)
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl<'a> Reader<'a> {
    /// Create a new reader.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            charged_to: usize::MAX,
            budget: None,
        }
    }

    /// Create a new reader that charges the bytes it reads to `budget`.
    ///
    /// When the budget refuses, the reader moves to the end of the data, so
    /// that the read fails as if the data had ended.
    #[inline]
    pub fn new_with_budget(data: &'a [u8], budget: Option<&'a dyn ParseBudget>) -> Self {
        let mut reader = Self::new(data);
        reader.attach_budget(budget);
        reader
    }

    /// Charges the bytes this reader reads from now on to `budget`, unless
    /// it already charges a budget.
    #[inline]
    pub(crate) fn attach_budget(&mut self, budget: Option<&'a dyn ParseBudget>) {
        if self.budget.is_none() && budget.is_some() {
            self.budget = budget;
            self.charged_to = self.offset;
        }
    }

    /// The budget this reader charges, if any.
    #[inline]
    pub(crate) fn budget(&self) -> Option<&'a dyn ParseBudget> {
        self.budget
    }

    /// Charges the bytes before `end` if they were not charged yet. If the
    /// budget refuses, moves to the end of the data and returns `None`.
    #[inline]
    pub(crate) fn charge(&mut self, end: usize) -> Option<()> {
        if end <= self.charged_to {
            Some(())
        } else {
            self.charge_cold(end)
        }
    }

    #[cold]
    fn charge_cold(&mut self, end: usize) -> Option<()> {
        let Some(budget) = self.budget else {
            return Some(());
        };
        let to = end
            .max(self.charged_to.saturating_add(WORK_WINDOW))
            .min(self.data.len());

        if budget.work(to - self.charged_to).is_err() {
            self.jump_to_end();
            return None;
        }

        self.charged_to = to;
        Some(())
    }

    /// Create a new reader at the given offset.
    #[inline]
    pub fn new_with(data: &'a [u8], offset: usize) -> Self {
        Self {
            data,
            offset,
            charged_to: usize::MAX,
            budget: None,
        }
    }

    /// Returns `true` if the reader has reached the end of the data.
    #[inline]
    pub fn at_end(&self) -> bool {
        self.offset >= self.data.len()
    }

    /// Moves the reader offset to the end of the data.
    #[inline]
    pub fn jump_to_end(&mut self) {
        self.offset = self.data.len();
    }

    /// Moves the reader to the specified offset.
    #[inline]
    pub fn jump(&mut self, offset: usize) {
        self.offset = offset;
    }

    /// Returns the remaining data from the current offset to the end.
    #[inline]
    pub fn tail(&mut self) -> Option<&'a [u8]> {
        self.data.get(self.offset..)
    }

    /// Returns the total length of the underlying data.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the underlying data is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns a slice of the data for the specified range.
    #[inline]
    pub fn range(&self, range: Range<usize>) -> Option<&'a [u8]> {
        self.data.get(range)
    }

    /// Returns the current offset of the reader.
    #[inline]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Reads the specified number of bytes and advances the offset.
    #[inline]
    pub fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let v = self.peek_bytes(len)?;
        self.charge(self.offset + len)?;
        self.offset += len;

        Some(v)
    }

    /// Reads a single byte and advances the offset.
    #[inline]
    pub fn read_byte(&mut self) -> Option<u8> {
        let v = self.peek_byte()?;
        self.charge(self.offset + 1)?;
        self.offset += 1;

        Some(v)
    }

    /// Skips the specified number of bytes by advancing the offset.
    #[inline]
    pub fn skip_bytes(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| {})
    }

    /// Peeks the specified number of bytes.
    #[inline]
    pub fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        self.offset
            .checked_add(len)
            .and_then(|end| self.data.get(self.offset..end))
    }

    /// Peeks a single byte.
    #[inline]
    pub fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.offset).copied()
    }

    /// Eat the next byte if it satisfies the condition.
    #[inline]
    pub fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        let val = self.peek_byte()?;
        if f(val) {
            self.forward();
            Some(val)
        } else {
            None
        }
    }

    /// Advances the offset by one byte.
    #[inline]
    pub fn forward(&mut self) {
        if self.charge(self.offset + 1).is_some() {
            self.offset += 1;
        }
    }

    /// Advances the offset by one byte if the current byte satisfies the predicate.
    #[inline]
    pub fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        if f(self.peek_byte()?) {
            self.forward();

            Some(())
        } else {
            None
        }
    }

    /// Advances the offset while bytes satisfy the predicate, at least one time.
    #[inline]
    pub fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        self.eat(&f)?;
        self.forward_while(f);
        Some(())
    }

    /// Advances the offset if the next bytes match the specified tag.
    #[inline]
    pub fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        self.peek_tag(tag)?;
        self.charge(self.offset + tag.len())?;
        self.offset += tag.len();

        Some(())
    }

    /// Advances the offset while the given byte satisfies the predicate.
    #[inline]
    pub fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        while let Some(b) = self.peek_byte() {
            if f(b) {
                self.forward();
            } else {
                break;
            }
        }
    }

    /// Checks if the next bytes match the specified tag.
    #[inline]
    pub fn peek_tag(&self, tag: &[u8]) -> Option<()> {
        self.data.get(self.offset..)?.starts_with(tag).then_some(())
    }

    /// Read a u16 integer (in big endian order).
    #[inline]
    pub fn read_u16(&mut self) -> Option<u16> {
        let bytes = self.read_bytes(2)?;

        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Read a u32 integer (in big endian order).
    #[inline]
    pub fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.read_bytes(4)?;

        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a u64 integer (in big endian order).
    #[inline]
    pub fn read_u64(&mut self) -> Option<u64> {
        let bytes = self.read_bytes(8)?;

        Some(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::Reader;

    #[test]
    fn peek_bytes_rejects_overflowing_len() {
        let reader = Reader::new(b"abc");
        assert!(reader.peek_bytes(usize::MAX).is_none());
    }
}
