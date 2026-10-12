//! Bit-level reader for CCITT encoded data streams.

use crate::{DecodeError, Result};
use core::fmt::Debug;

#[derive(Debug, Clone)]
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    bit_offset: usize,
    /// The bits starting at `bit_offset`, with the next bit in the most
    /// significant position. Always contains at least 32 valid bits, bits past
    /// the end of the data are zero.
    buffer: u64,
    /// The bit offset up to which bits can be consumed without refilling the
    /// buffer or reaching the end of the data.
    limit: usize,
}

impl<'a> BitReader<'a> {
    #[inline(always)]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut reader = Self {
            data,
            bit_offset: 0,
            buffer: 0,
            limit: 0,
        };
        reader.refill();
        reader
    }

    #[inline(always)]
    pub(crate) fn read_bit(&mut self) -> Result<u32> {
        let bit = (self.buffer >> 63) as u32;
        self.consume(1)?;

        Ok(bit)
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
        (self.buffer >> 32) as u32
    }

    /// Consume up to 32 bits.
    #[inline(always)]
    pub(crate) fn consume(&mut self, num_bits: usize) -> Result<()> {
        let bit_offset = self.bit_offset + num_bits;

        if bit_offset > self.limit {
            return self.consume_slow(bit_offset);
        }

        self.bit_offset = bit_offset;
        self.buffer <<= num_bits;

        Ok(())
    }

    #[inline(always)]
    fn consume_slow(&mut self, bit_offset: usize) -> Result<()> {
        let end = self.data.len() * 8;
        self.bit_offset = bit_offset.min(end);
        self.refill();

        if bit_offset > end {
            Err(DecodeError::UnexpectedEof)
        } else {
            Ok(())
        }
    }

    #[inline(always)]
    pub(crate) fn align(&mut self) {
        let bit_pos = self.bit_pos();

        if !bit_pos.is_multiple_of(8) {
            // The end of the data is always byte-aligned, so this can't fail.
            let _ = self.consume(8 - bit_pos);
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

    #[inline(always)]
    fn refill(&mut self) {
        let byte_pos = self.byte_pos();

        let word = if let Some(bytes) = self.data.get(byte_pos..byte_pos + 8) {
            u64::from_be_bytes(bytes.try_into().unwrap())
        } else {
            let mut bytes = [0; 8];
            let rest = self.data.get(byte_pos..).unwrap_or_default();
            bytes[..rest.len()].copy_from_slice(rest);
            u64::from_be_bytes(bytes)
        };

        self.buffer = word << self.bit_pos();
        // The buffer contains bits up to `byte_pos * 8 + 64`. Refill once fewer
        // than 32 of them are left.
        self.limit = (byte_pos * 8 + 32).min(self.data.len() * 8);
    }
}
