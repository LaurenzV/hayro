//! A small and compact bit reader.

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

    #[inline]
    pub(crate) fn tail(&self) -> &'a [u8] {
        &self.data[self.byte_pos()..]
    }

    /// Like the normal `read_bits` method, but accounts for stuffing bits
    /// in addition.
    #[inline]
    pub(crate) fn read_bits_with_stuffing(&mut self, bit_size: u8) -> Option<u32> {
        let mut bit = 0;

        for _ in 0..bit_size {
            self.read_stuff_bit_if_necessary()?;
            bit = (bit << 1) | self.read_bit()?;
        }

        Some(bit)
    }

    #[inline]
    pub(crate) fn read_stuff_bit_if_necessary(&mut self) -> Option<()> {
        // B.10.1: "If the value of the byte is 0xFF, the next byte includes an extra zero bit
        // stuffed into the MSB.
        // Check if the next bit is at a new byte boundary."
        if self.bit_pos() == 0 && self.byte_pos() > 0 {
            let last_byte = self.data[self.byte_pos() - 1];

            if last_byte == 0xff {
                let stuff_bit = self.read_bit()?;

                if stuff_bit != 0 {
                    return None;
                }
            }
        }

        Some(())
    }

    #[inline]
    pub(crate) fn peak_bits_with_stuffing(&mut self, bit_size: u8) -> Option<u32> {
        self.clone().read_bits_with_stuffing(bit_size)
    }
}
