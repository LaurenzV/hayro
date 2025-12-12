//! Combined byte and bit reader utilities.

use std::fmt::Debug;

#[derive(Debug, Clone)]
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    cur_pos: usize,
}

impl<'a> BitReader<'a> {
    #[inline]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, cur_pos: 0 }
    }

    #[inline]
    pub(crate) fn align(&mut self) {
        let bit_pos = self.bit_pos();

        if !bit_pos.is_multiple_of(8) {
            self.cur_pos += 8 - bit_pos;
        }
    }

    #[inline]
    pub(crate) fn at_end(&self) -> bool {
        self.byte_pos() >= self.data.len()
    }

    #[inline]
    pub(crate) fn jump_to_end(&mut self) {
        self.cur_pos = self.data.len() * 8;
    }
    #[inline]
    pub(crate) fn offset(&self) -> usize {
        self.byte_pos()
    }


    #[inline(always)]
    pub(crate) fn read_bit(&mut self) -> Option<u32> {
        let byte_pos = self.byte_pos();
        let byte = *self.data.get(byte_pos)? as u32;
        let shift = 7 - self.bit_pos();
        self.cur_pos += 1;
        Some((byte >> shift) & 1)
    }

    #[inline]
    pub(crate) fn byte_pos(&self) -> usize {
        self.cur_pos / 8
    }

    #[inline]
    pub(crate) fn bit_pos(&self) -> usize {
        self.cur_pos % 8
    }
}
