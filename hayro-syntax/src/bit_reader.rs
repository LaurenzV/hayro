//! A bit reader that supports reading numbers from a bit stream, with a number of bits
//! up to 32.

use log::warn;
use smallvec::{SmallVec, smallvec};
use std::fmt::Debug;

/// A bit size.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub struct BitSize(u8);

impl BitSize {
    /// Create a new `BitSize`. Returns `None` if the number is bigger than 32.
    pub fn from_u8(value: u8) -> Option<Self> {
        if value > 32 { None } else { Some(Self(value)) }
    }

    /// Return the number of bits.
    pub fn bits(&self) -> usize {
        self.0 as usize
    }

    /// Return the bit mask.
    pub fn mask(&self) -> u32 {
        ((1u64 << self.0 as u64) - 1) as u32
    }
}

/// A bit reader.
pub struct BitReader<'a> {
    data: &'a [u8],
    cur_pos: usize,
}

impl<'a> BitReader<'a> {
    /// Create a new bit reader.
    pub fn new(data: &'a [u8]) -> Self {
        Self::new_with(data, 0)
    }

    /// Create a new bit reader, and start at a specific bit offset.
    pub fn new_with(data: &'a [u8], cur_pos: usize) -> Self {
        Self { data, cur_pos }
    }

    /// Align the reader to the next byte boundary.
    pub fn align(&mut self) {
        let bit_pos = self.bit_pos();

        if !bit_pos.is_multiple_of(8) {
            self.cur_pos += 8 - bit_pos;
        }
    }

    /// Read the given number of bits from the byte stream.
    pub fn read(&mut self, bit_size: BitSize) -> Option<u32> {
        let byte_pos = self.byte_pos();

        if bit_size.0 > 32 || byte_pos >= self.data.len() {
            return None;
        }

        let item = match bit_size.0 {
            8 => {
                let item = self.data[byte_pos] as u32;
                self.cur_pos += 8;

                Some(item)
            }
            0..=32 => {
                let bit_pos = self.bit_pos();
                let end_byte_pos = (bit_pos + bit_size.0 as usize - 1) / 8;
                let mut read = [0u8; 8];

                for (i, r) in read.iter_mut().enumerate().take(end_byte_pos + 1) {
                    *r = *self.data.get(byte_pos + i)?;
                }

                let item = (u64::from_be_bytes(read) >> (64 - bit_pos - bit_size.0 as usize))
                    as u32
                    & bit_size.mask();
                self.cur_pos += bit_size.0 as usize;

                Some(item)
            }
            _ => unreachable!(),
        }?;

        Some(item)
    }

    fn byte_pos(&self) -> usize {
        self.cur_pos / 8
    }

    fn bit_pos(&self) -> usize {
        self.cur_pos % 8
    }
}

#[derive(Debug)]
pub(crate) struct BitWriter<'a> {
    data: &'a mut [u8],
    cur_pos: usize,
    bit_size: BitSize,
}

impl<'a> BitWriter<'a> {
    pub(crate) fn new(data: &'a mut [u8], bit_size: BitSize) -> Option<Self> {
        if !matches!(bit_size.0, 1 | 2 | 4 | 8 | 16) {
            return None;
        }

        Some(Self {
            data,
            bit_size,
            cur_pos: 0,
        })
    }

    pub(crate) fn split_off(self) -> (&'a [u8], BitWriter<'a>) {
        // Assumes that we are currently aligned to a byte boundary!
        let (left, right) = self.data.split_at_mut(self.cur_pos / 8);
        (
            left,
            BitWriter {
                data: right,
                cur_pos: 0,
                bit_size: self.bit_size,
            },
        )
    }

    /// Align the writer to the next byte boundary.
    #[cfg(feature = "jpeg2000")]
    pub(crate) fn align(&mut self) {
        let bit_pos = self.bit_pos();

        if !bit_pos.is_multiple_of(8) {
            self.cur_pos += 8 - bit_pos;
        }
    }

    pub(crate) fn cur_pos(&self) -> usize {
        self.cur_pos
    }

    pub(crate) fn get_data(&self) -> &[u8] {
        self.data
    }

    fn byte_pos(&self) -> usize {
        self.cur_pos / 8
    }

    fn bit_pos(&self) -> usize {
        self.cur_pos % 8
    }

    pub(crate) fn write(&mut self, val: u16) -> Option<()> {
        let byte_pos = self.byte_pos();
        let bit_size = self.bit_size;

        match bit_size.0 {
            1 | 2 | 4 => {
                let bit_pos = self.bit_pos();

                let base = self.data.get(byte_pos)?;
                let shift = 8 - self.bit_size.bits() - bit_pos;
                let item = ((val & self.bit_size.mask() as u16) as u8) << shift;

                *(self.data.get_mut(byte_pos)?) = *base | item;
                self.cur_pos += bit_size.bits();
            }
            8 => {
                *(self.data.get_mut(byte_pos)?) = val as u8;
                self.cur_pos += 8;
            }
            16 => {
                self.data
                    .get_mut(byte_pos..(byte_pos + 2))?
                    .copy_from_slice(&val.to_be_bytes());
                self.cur_pos += 16;
            }
            _ => unreachable!(),
        }

        Some(())
    }
}

pub(crate) struct BitChunks<'a> {
    reader: BitReader<'a>,
    bit_size: BitSize,
    chunk_len: usize,
}

impl<'a> BitChunks<'a> {
    pub(crate) fn new(data: &'a [u8], bit_size: BitSize, chunk_len: usize) -> Option<Self> {
        if bit_size.0 > 16 {
            warn!("BitChunks doesn't support working with bit sizes > 16.");

            return None;
        }

        let reader = BitReader::new(data);

        Some(Self {
            reader,
            bit_size,
            chunk_len,
        })
    }
}

impl Iterator for BitChunks<'_> {
    type Item = BitChunk;

    fn next(&mut self) -> Option<Self::Item> {
        let mut bits = SmallVec::new();

        for _ in 0..self.chunk_len {
            bits.push(self.reader.read(self.bit_size)? as u16);
        }

        Some(BitChunk { bits })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BitChunk {
    bits: SmallVec<[u16; 4]>,
}

impl BitChunk {
    pub(crate) fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.bits.iter().copied()
    }

    pub(crate) fn new(val: u8, count: usize) -> Self {
        Self {
            bits: smallvec![val as u16; count],
        }
    }

    pub(crate) fn from_reader(
        bit_reader: &mut BitReader,
        bit_size: BitSize,
        chunk_len: usize,
    ) -> Option<Self> {
        if bit_size.0 > 16 {
            warn!("BitChunk doesn't support working with bit sizes > 16.");

            return None;
        }

        let mut bits = SmallVec::new();

        for _ in 0..chunk_len {
            bits.push(bit_reader.read(bit_size)? as u16);
        }

        Some(BitChunk { bits })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BS1: BitSize = BitSize(1);
    const BS2: BitSize = BitSize(2);
    const BS4: BitSize = BitSize(4);
    const BS8: BitSize = BitSize(8);
    const BS16: BitSize = BitSize(16);

    #[test]
    fn bit_reader_16() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let mut reader = BitReader::new(&data);
        assert_eq!(
            reader.read(BS16).unwrap() as u16,
            u16::from_be_bytes([0x01, 0x02])
        );
        assert_eq!(
            reader.read(BS16).unwrap() as u16,
            u16::from_be_bytes([0x03, 0x04])
        );
        assert_eq!(
            reader.read(BS16).unwrap() as u16,
            u16::from_be_bytes([0x05, 0x06])
        );
    }

    #[test]
    fn bit_writer_16() {
        let mut buf = vec![0u8; 6];
        let mut writer = BitWriter::new(&mut buf, BitSize::from_u8(16).unwrap()).unwrap();
        writer.write(u16::from_be_bytes([0x01, 0x02])).unwrap();
        writer.write(u16::from_be_bytes([0x03, 0x04])).unwrap();
        writer.write(u16::from_be_bytes([0x05, 0x06])).unwrap();

        assert_eq!(buf, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    }

    #[test]
    fn bit_reader_12() {
        let data = [0b10011000, 0b00011111, 0b10101001, 0b11101001, 0b00011010];
        let mut reader = BitReader::new(&data);
        assert_eq!(
            reader.read(BitSize::from_u8(12).unwrap()).unwrap(),
            0b100110000001
        );
        assert_eq!(
            reader.read(BitSize::from_u8(12).unwrap()).unwrap(),
            0b111110101001
        );
        assert_eq!(
            reader.read(BitSize::from_u8(12).unwrap()).unwrap(),
            0b111010010001
        );
    }

    #[test]
    fn bit_reader_9() {
        let data = [0b10011000, 0b00011111, 0b10101001, 0b11101001, 0b00011010];
        let mut reader = BitReader::new(&data);
        assert_eq!(
            reader.read(BitSize::from_u8(9).unwrap()).unwrap(),
            0b100110000
        );
        assert_eq!(
            reader.read(BitSize::from_u8(9).unwrap()).unwrap(),
            0b001111110
        );
        assert_eq!(
            reader.read(BitSize::from_u8(9).unwrap()).unwrap(),
            0b101001111
        );
        assert_eq!(
            reader.read(BitSize::from_u8(9).unwrap()).unwrap(),
            0b010010001
        );
    }

    #[test]
    fn bit_writer_8() {
        let mut buf = vec![0u8; 3];
        let mut writer = BitWriter::new(&mut buf, BitSize::from_u8(8).unwrap()).unwrap();
        writer.write(0x01).unwrap();
        writer.write(0x02).unwrap();
        writer.write(0x03).unwrap();

        assert_eq!(buf, [0x01, 0x02, 0x03]);
    }

    #[test]
    fn bit_reader_8() {
        let data = [0x01, 0x02, 0x03];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(BS8).unwrap(), 0x01);
        assert_eq!(reader.read(BS8).unwrap(), 0x02);
        assert_eq!(reader.read(BS8).unwrap(), 0x03);
    }

    #[test]
    fn bit_writer_4() {
        let mut buf = vec![0u8; 3];
        let mut writer = BitWriter::new(&mut buf, BitSize::from_u8(4).unwrap()).unwrap();
        writer.write(0b1001).unwrap();
        writer.write(0b1000).unwrap();
        writer.write(0b0001).unwrap();
        writer.write(0b1111).unwrap();
        writer.write(0b1010).unwrap();
        writer.write(0b1001).unwrap();

        assert_eq!(buf, [0b10011000, 0b00011111, 0b10101001]);
    }

    #[test]
    fn bit_reader_4() {
        let data = [0b10011000, 0b00011111, 0b10101001];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(BS4).unwrap(), 0b1001);
        assert_eq!(reader.read(BS4).unwrap(), 0b1000);
        assert_eq!(reader.read(BS4).unwrap(), 0b0001);
        assert_eq!(reader.read(BS4).unwrap(), 0b1111);
        assert_eq!(reader.read(BS4).unwrap(), 0b1010);
        assert_eq!(reader.read(BS4).unwrap(), 0b1001);
    }

    #[test]
    fn bit_writer_2() {
        let mut buf = vec![0u8; 2];
        let mut writer = BitWriter::new(&mut buf, BitSize::from_u8(2).unwrap()).unwrap();
        writer.write(0b10).unwrap();
        writer.write(0b01).unwrap();
        writer.write(0b10).unwrap();
        writer.write(0b00).unwrap();
        writer.write(0b00).unwrap();
        writer.write(0b01).unwrap();
        writer.write(0b00).unwrap();
        writer.write(0b00).unwrap();

        assert_eq!(buf, [0b10011000, 0b00010000]);
    }

    #[test]
    fn bit_reader_2() {
        let data = [0b10011000, 0b00010000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(BS2).unwrap(), 0b10);
        assert_eq!(reader.read(BS2).unwrap(), 0b01);
        assert_eq!(reader.read(BS2).unwrap(), 0b10);
        assert_eq!(reader.read(BS2).unwrap(), 0b00);
        assert_eq!(reader.read(BS2).unwrap(), 0b00);
        assert_eq!(reader.read(BS2).unwrap(), 0b01);
        assert_eq!(reader.read(BS2).unwrap(), 0b00);
        assert_eq!(reader.read(BS2).unwrap(), 0b00);
    }

    #[test]
    fn bit_writer_1() {
        let mut buf = vec![0u8; 2];
        let mut writer = BitWriter::new(&mut buf, BitSize::from_u8(1).unwrap()).unwrap();
        writer.write(0b1).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b1).unwrap();
        writer.write(0b1).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();

        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b1).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();
        writer.write(0b0).unwrap();

        assert_eq!(buf, [0b10011000, 0b00010000]);
    }

    #[test]
    fn bit_reader_1() {
        let data = [0b10011000, 0b00010000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);

        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
    }

    #[test]
    fn bit_reader_align() {
        let data = [0b10011000, 0b00010000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        reader.align();

        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
        assert_eq!(reader.read(BS1).unwrap(), 0b0);
    }

    #[test]
    fn bit_reader_chunks() {
        let data = [0b10011000, 0b00010000];
        let mut reader = BitChunks::new(&data, BitSize::from_u8(1).unwrap(), 3).unwrap();
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b1, 0b0, 0b0]);
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b1, 0b1, 0b0]);
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b0, 0b0, 0b0]);
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b0, 0b0, 0b1]);
        assert_eq!(reader.next().unwrap().bits.as_ref(), &[0b0, 0b0, 0b0]);
    }

    #[test]
    fn bit_reader_varying_bit_sizes() {
        let data = [0b10011000, 0b00011111, 0b10101001];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read(BS4).unwrap(), 0b1001);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        assert_eq!(reader.read(BS4).unwrap(), 0b0000);
        assert_eq!(reader.read(BitSize::from_u8(5).unwrap()).unwrap(), 0b00111);
        assert_eq!(reader.read(BS1).unwrap(), 0b1);
        assert_eq!(reader.read(BS2).unwrap(), 0b11);
        assert_eq!(
            reader.read(BitSize::from_u8(7).unwrap()).unwrap(),
            0b0101001
        );
    }
}
