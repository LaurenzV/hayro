//! Bit-level reader for CCITT encoded data streams.

use crate::{DecodeError, Result};
use core::fmt::Debug;

#[derive(Debug, Clone)]
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> BitReader<'a> {
    #[inline(always)]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    #[inline(always)]
    pub(crate) fn read_bit(&mut self) -> Result<u32> {
        let byte_pos = self.byte_pos();
        let byte = *self.data.get(byte_pos).ok_or(DecodeError::UnexpectedEof)? as u32;
        let shift = 7 - self.bit_pos();
        self.bit_offset += 1;
        Ok((byte >> shift) & 1)
    }

    #[inline(always)]
    pub(crate) fn read_bits(&mut self, num_bits: usize) -> Result<u32> {
        let mut result = 0_u32;

        for i in (0..num_bits).rev() {
            result |= (self.read_bit()?) << i;
        }

        Ok(result)
    }

    /// Return the next 32 bits without consuming them. Bits past the end of the
    /// data are zero.
    #[inline(always)]
    pub(crate) fn peek_u32(&self) -> u32 {
        let byte_pos = self.byte_pos();

        let word = if let Some(bytes) = self.data.get(byte_pos..byte_pos + 8) {
            u64::from_be_bytes(bytes.try_into().unwrap())
        } else {
            let mut bytes = [0; 8];
            let rest = self.data.get(byte_pos..).unwrap_or_default();
            bytes[..rest.len()].copy_from_slice(rest);
            u64::from_be_bytes(bytes)
        };

        ((word << self.bit_pos()) >> 32) as u32
    }

    /// Consume the given number of bits.
    #[inline(always)]
    pub(crate) fn consume(&mut self, num_bits: usize) -> Result<()> {
        let end = self.data.len() * 8;

        if self.bit_offset + num_bits > end {
            self.bit_offset = end;
            return Err(DecodeError::UnexpectedEof);
        }

        self.bit_offset += num_bits;

        Ok(())
    }

    #[inline(always)]
    pub(crate) fn align(&mut self) {
        let bit_pos = self.bit_pos();

        if !bit_pos.is_multiple_of(8) {
            self.bit_offset += 8 - bit_pos;
        }
    }

    #[inline(always)]
    pub(crate) fn at_end(&self) -> bool {
        self.byte_pos() >= self.data.len()
    }

    #[inline(always)]
    pub(crate) fn byte_pos(&self) -> usize {
        self.bit_offset >> 3
    }

    #[inline(always)]
    fn bit_pos(&self) -> usize {
        self.bit_offset & 7
    }
}
