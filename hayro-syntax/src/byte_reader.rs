//! A byte reader.

use core::ops::Range;
use std::io::Read;
use std::io::Seek;

use crate::util;

// impl<'a> From<Vec<u8>> for ReadBytes<'a> {
//     fn from(value: Vec<u8>) -> Self {
//         ReadBytes::Vec(value)
//     }
// }

// impl<'a> From<&'a [u8]> for ReadBytes<'a> {
//     fn from(value: &'a [u8]) -> Self {
//         ReadBytes::Slice(value)
//     }
// }

// impl<'a> AsRef<[u8]> for ReadBytes<'a> {
//     fn as_ref(&self) -> &[u8] {
//         match self {
//             ReadBytes::Slice(slice) => *slice,
//             ReadBytes::Vec(vec) => vec.as_slice(),
//         }
//     }
// }

pub trait ByteReader<'a> {
    /// Returns `true` if the reader has reached the end of the data.
    fn at_end(&self) -> bool;

    /// Moves the reader offset to the end of the data.
    fn jump_to_end(&mut self);

    /// Moves the reader to the specified offset.
    fn jump(&mut self, offset: usize);

    /// Returns the remaining data from the current offset to the end.
    fn tail(&mut self) -> Option<&'a [u8]>;

    /// Returns the total length of the underlying data.
    fn len(&self) -> usize;

    /// Returns `true` if the underlying data is empty.
    fn is_empty(&self) -> bool;

    /// Returns a slice of the data for the specified range.
    fn range(&self, range: Range<usize>) -> Option<&'a [u8]>;

    /// Returns the current offset of the reader.
    fn offset(&self) -> usize;

    /// Reads the specified number of bytes and advances the offset.
    fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]>;

    /// Reads a single byte and advances the offset.
    fn read_byte(&mut self) -> Option<u8>;

    /// Skips the specified number of bytes by advancing the offset.
    fn skip_bytes(&mut self, len: usize) -> Option<()>;

    /// Peeks the specified number of bytes.
    fn peek_bytes(&self, len: usize) -> Option<&'a [u8]>;

    /// Peeks a single byte.
    fn peek_byte(&self) -> Option<u8>;

    /// Eat the next byte if it satisfies the condition.
    fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8>;

    /// Advances the offset by one byte.
    fn forward(&mut self);

    /// Advances the offset by one byte if the current byte satisfies the predicate.
    fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()>;

    /// Advances the offset while bytes satisfy the predicate, at least one time.
    fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()>;

    /// Advances the offset if the next bytes match the specified tag.
    fn forward_tag(&mut self, tag: &[u8]) -> Option<()>;

    /// Advances the offset while the given byte satisfies the predicate.
    fn forward_while(&mut self, f: impl Fn(u8) -> bool);

    /// Checks if the next bytes match the specified tag.
    fn peek_tag(&self, tag: &[u8]) -> Option<()>;

    /// Read a u16 integer (in big endian order).
    fn read_u16(&mut self) -> Option<u16>;

    /// Read a u32 integer (in big endian order).
    fn read_u32(&mut self) -> Option<u32>;

    /// Read a u64 integer (in big endian order).
    fn read_u64(&mut self) -> Option<u64>;

    fn find_needle(&mut self, needle: &[u8]) -> Option<usize>;

    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize>;
}

#[derive(Clone, Debug)]
pub enum Reader<'a> {
    Slice(SliceReader<'a>),
}

impl <'a> Reader<'a> {
    pub fn from_slice(slice: &'a [u8]) -> Self {
        Reader::Slice(SliceReader::new(slice))
    }
}

impl<'a> ByteReader<'a> for Reader<'a> {
    #[inline]
    fn at_end(&self) -> bool {
        match self {
            Reader::Slice(reader) => reader.at_end(),
        }
    }

    #[inline]
    fn jump_to_end(&mut self) {
        match self {
            Reader::Slice(reader) => reader.jump_to_end(),
        }
    }

    #[inline]
    fn jump(&mut self, offset: usize) {
        match self {
            Reader::Slice(reader) => reader.jump(offset),
        }
    }

    #[inline]
    fn tail(&mut self) -> Option<&'a [u8]> {
        match self {
            Reader::Slice(reader) => reader.tail(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            Reader::Slice(reader) => reader.len(),
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        match self {
            Reader::Slice(reader) => reader.is_empty(),
        }
    }

    #[inline]
    fn range(&self, range: Range<usize>) -> Option<&'a [u8]> {
        match self {
            Reader::Slice(reader) => reader.range(range),
        }
    }

    #[inline]
    fn offset(&self) -> usize {
        match self {
            Reader::Slice(reader) => reader.offset(),
        }
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        match self {
            Reader::Slice(reader) => reader.read_bytes(len),
        }
    }

    #[inline]
    fn read_byte(&mut self) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.read_byte(),
        }
    }

    #[inline]
    fn skip_bytes(&mut self, len: usize) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.skip_bytes(len),
        }
    }

    #[inline]
    fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        match self {
            Reader::Slice(reader) => reader.peek_bytes(len),
        }
    }

    #[inline]
    fn peek_byte(&self) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.peek_byte(),
        }
    }

    #[inline]
    fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.eat(f),
        }
    }

    #[inline]
    fn forward(&mut self) {
        match self {
            Reader::Slice(reader) => reader.forward(),
        }
    }

    #[inline]
    fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_if(f),
        }
    }

    #[inline]
    fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_while_1(f),
        }
    }

    #[inline]
    fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_tag(tag),
        }
    }

    #[inline]
    fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        match self {
            Reader::Slice(reader) => reader.forward_while(f),
        }
    }

    #[inline]
    fn peek_tag(&self, tag: &[u8]) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.peek_tag(tag),
        }
    }

    #[inline]
    fn read_u16(&mut self) -> Option<u16> {
        match self {
            Reader::Slice(reader) => reader.read_u16(),
        }
    }

    #[inline]
    fn read_u32(&mut self) -> Option<u32> {
        match self {
            Reader::Slice(reader) => reader.read_u32(),
        }
    }

    #[inline]
    fn read_u64(&mut self) -> Option<u64> {
        match self {
            Reader::Slice(reader) => reader.read_u64(),
        }
    }
    
    fn find_needle(&mut self, needle: &[u8]) -> Option<usize> {
        match self {
            Reader::Slice(reader) => reader.find_needle(needle),
        }
    }
    
    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize> {
        match self {
            Reader::Slice(reader) => reader.findr_needle(needle),
        }
    }
}

/// A reader for reading bytes and PDF objects.
#[derive(Clone, Debug)]
pub struct SliceReader<'a> {
    /// The underlying data of the reader.
    pub data: &'a [u8],
    /// The current byte-offset.
    pub offset: usize,
}

impl<'a> SliceReader<'a> {
    /// Create a new reader.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    /// Create a new reader at the given offset.
    #[inline]
    pub fn new_with(data: &'a [u8], offset: usize) -> Self {
        Self { data, offset }
    }
}

impl<'a> ByteReader<'a> for SliceReader<'a> {
    #[inline]
    fn at_end(&self) -> bool {
        self.offset >= self.data.len()
    }

    #[inline]
    fn jump_to_end(&mut self) {
        self.offset = self.data.len();
    }

    #[inline]
    fn jump(&mut self, offset: usize) {
        self.offset = offset;
    }

    #[inline]
    fn tail(&mut self) -> Option<&'a [u8]> {
        self.data.get(self.offset..).map(|s| s.into())
    }

    #[inline]
    fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[inline]
    fn range(&self, range: Range<usize>) -> Option<&'a [u8]> {
        self.data.get(range).map(|s| s.into())
    }

    #[inline]
    fn offset(&self) -> usize {
        self.offset
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let v = self.peek_bytes(len)?;
        self.offset += len;

        Some(v)
    }

    #[inline]
    fn read_byte(&mut self) -> Option<u8> {
        let v = self.peek_byte()?;
        self.offset += 1;

        Some(v)
    }

    #[inline]
    fn skip_bytes(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| {})
    }

    #[inline]
    fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        self.offset
            .checked_add(len)
            .and_then(|end| self.data.get(self.offset..end))
            .map(|s| s.into())
    }

    #[inline]
    fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.offset).copied()
    }

    #[inline]
    fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        let val = self.peek_byte()?;
        if f(val) {
            self.forward();
            Some(val)
        } else {
            None
        }
    }

    #[inline]
    fn forward(&mut self) {
        self.offset += 1;
    }

    #[inline]
    fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        if f(self.peek_byte()?) {
            self.forward();

            Some(())
        } else {
            None
        }
    }

    #[inline]
    fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        self.eat(&f)?;
        self.forward_while(f);
        Some(())
    }

    #[inline]
    fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        self.peek_tag(tag)?;
        self.offset += tag.len();

        Some(())
    }

    #[inline]
    fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        while let Some(b) = self.peek_byte() {
            if f(b) {
                self.forward();
            } else {
                break;
            }
        }
    }

    #[inline]
    fn peek_tag(&self, tag: &[u8]) -> Option<()> {
        let mut cloned = self.clone();

        for b in tag.iter().copied() {
            if cloned.peek_byte() == Some(b) {
                cloned.forward();
            } else {
                return None;
            }
        }

        Some(())
    }

    #[inline]
    fn read_u16(&mut self) -> Option<u16> {
        let read_bytes = self.read_bytes(2)?;
        let bytes = read_bytes.as_ref();

        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    #[inline]
    fn read_u32(&mut self) -> Option<u32> {
        let read_bytes = self.read_bytes(4)?;
        let bytes = read_bytes.as_ref();

        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    #[inline]
    fn read_u64(&mut self) -> Option<u64> {
        let read_bytes = self.read_bytes(8)?;
        let bytes = read_bytes.as_ref();

        Some(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
    
    fn find_needle(&mut self, needle: &[u8]) -> Option<usize> {
        util::find_needle(self.data, needle)
    }
    
    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize> {
        util::findr_needle(self.data, needle)
    }
}

pub trait SeekRead: Seek + Read {}

#[cfg(test)]
mod tests {
    use super::{ByteReader, SliceReader};

    #[test]
    fn peek_bytes_rejects_overflowing_len() {
        let reader = SliceReader::new(b"abc");
        assert!(reader.peek_bytes(usize::MAX).is_none());
    }
}
