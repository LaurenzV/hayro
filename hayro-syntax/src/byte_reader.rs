//! A byte reader.

use core::fmt::Debug;
use core::ops::Deref;
use core::ops::Range;
use core::ops::RangeBounds;
use std::sync::Mutex;

use alloc::borrow::Cow;
use alloc::sync::Arc;

use crate::util;

/// Struct holding bytes that have been read
#[derive(Clone, Debug)]
pub struct ReadBytes<'a>(Cow<'a, [u8]>);

impl<'a> ReadBytes<'a> {
    pub const EMPTY: ReadBytes<'static> = ReadBytes(Cow::Borrowed(&[]));

    #[inline]
    pub fn inner(&self) -> &Cow<'a, [u8]> {
        &self.0
    }

    #[inline]
    pub fn clone_range<B: RangeBounds<usize>>(&self, range: B) -> Self {
        let range = range_from_bounds(range, self.len());
        match &self.0 {
            Cow::Borrowed(data) => ReadBytes(Cow::Borrowed(&data[range])),
            Cow::Owned(data) => ReadBytes(Cow::Owned(data[range].to_vec())),
        }
    }
}

impl<'a> Deref for ReadBytes<'a> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<'a> PartialEq for ReadBytes<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<'a> AsRef<[u8]> for ReadBytes<'a> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<'a> From<&'a [u8]> for ReadBytes<'a> {
    #[inline]
    fn from(value: &'a [u8]) -> Self {
        Self(Cow::Borrowed(value))
    }
}

impl<'a, const C: usize> From<&'a [u8; C]> for ReadBytes<'a> {
    #[inline]
    fn from(value: &'a [u8; C]) -> Self {
        Self(Cow::Borrowed(value))
    }
}

impl<'a> From<Vec<u8>> for ReadBytes<'a> {
    #[inline]
    fn from(value: Vec<u8>) -> Self {
        Self(Cow::Owned(value))
    }
}

impl<'a> Into<Cow<'a, [u8]>> for ReadBytes<'a> {
    #[inline]
    fn into(self) -> Cow<'a, [u8]> {
        self.0
    }
}

pub trait ByteReader<'a> {
    /// Returns `true` if the reader has reached the end of the data.
    fn at_end(&self) -> bool;

    /// Moves the reader offset to the end of the data.
    fn jump_to_end(&mut self);

    /// Moves the reader to the specified offset.
    fn jump(&mut self, offset: usize);

    /// Returns the total length of the underlying data.
    fn len(&self) -> usize;

    /// Returns `true` if the underlying data is empty.
    fn is_empty(&self) -> bool;

    /// Returns a slice of the data for the specified range.
    fn range(&self, range: Range<usize>) -> Option<ReadBytes<'a>>;

    /// Returns the current offset of the reader.
    fn offset(&self) -> usize;

    /// Reads the specified number of bytes and advances the offset.
    fn read_bytes(&mut self, len: usize) -> Option<ReadBytes<'a>>;

    /// Reads a single byte and advances the offset.
    fn read_byte(&mut self) -> Option<u8>;

    /// Skips the specified number of bytes by advancing the offset.
    fn skip_bytes(&mut self, len: usize) -> Option<()>;

    /// Peeks the specified number of bytes.
    fn peek_bytes(&self, len: usize) -> Option<ReadBytes<'a>>;

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

    // Returns the index of the first occurrence of the given needle
    fn find_needle(&mut self, needle: &[u8]) -> Option<usize>;

    // Returns the index of the last occurrence of the given needle
    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize>;
}

#[derive(Clone, Debug)]
pub enum Reader<'a> {
    Slice(SliceReader<'a>),
    Custom(CustomReader),
}

impl <'a> Reader<'a> {
    pub fn from_read(read: ReadBytes<'a>) -> Self {
        Reader::Slice(SliceReader::new(read))
    }
    pub fn from_slice(slice: &'a [u8]) -> Self {
        Reader::Slice(SliceReader::new(slice.into()))
    }
    pub fn from_custom_source(read_seek: Arc<Mutex<dyn CustomSource>>) -> Self {
        Reader::Custom(CustomReader::new(read_seek))
    }
}

impl<'a> ByteReader<'a> for Reader<'a> {
    #[inline]
    fn at_end(&self) -> bool {
        match self {
            Reader::Slice(reader) => reader.at_end(),
            Reader::Custom(reader) => reader.at_end(),
        }
    }

    #[inline]
    fn jump_to_end(&mut self) {
        match self {
            Reader::Slice(reader) => reader.jump_to_end(),
            Reader::Custom(reader) => reader.jump_to_end(),
        }
    }

    #[inline]
    fn jump(&mut self, offset: usize) {
        match self {
            Reader::Slice(reader) => reader.jump(offset),
            Reader::Custom(reader) => reader.jump(offset),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            Reader::Slice(reader) => reader.len(),
            Reader::Custom(reader) => reader.len(),
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        match self {
            Reader::Slice(reader) => reader.is_empty(),
            Reader::Custom(reader) => reader.is_empty(),
        }
    }

    #[inline]
    fn range(&self, range: Range<usize>) -> Option<ReadBytes<'a>> {
        match self {
            Reader::Slice(reader) => reader.range(range),
            Reader::Custom(reader) => reader.range(range),
        }
    }

    #[inline]
    fn offset(&self) -> usize {
        match self {
            Reader::Slice(reader) => reader.offset(),
            Reader::Custom(reader) => reader.offset(),
        }
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<ReadBytes<'a>> {
        match self {
            Reader::Slice(reader) => reader.read_bytes(len),
            Reader::Custom(reader) => reader.read_bytes(len),
        }
    }

    #[inline]
    fn read_byte(&mut self) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.read_byte(),
            Reader::Custom(reader) => reader.read_byte(),
        }
    }

    #[inline]
    fn skip_bytes(&mut self, len: usize) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.skip_bytes(len),
            Reader::Custom(reader) => reader.skip_bytes(len),
        }
    }

    #[inline]
    fn peek_bytes(&self, len: usize) -> Option<ReadBytes<'a>> {
        match self {
            Reader::Slice(reader) => reader.peek_bytes(len),
            Reader::Custom(reader) => reader.peek_bytes(len),
        }
    }

    #[inline]
    fn peek_byte(&self) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.peek_byte(),
            Reader::Custom(reader) => reader.peek_byte(),
        }
    }

    #[inline]
    fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.eat(f),
            Reader::Custom(reader) => reader.eat(f),
        }
    }

    #[inline]
    fn forward(&mut self) {
        match self {
            Reader::Slice(reader) => reader.forward(),
            Reader::Custom(reader) => reader.forward(),
        }
    }

    #[inline]
    fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_if(f),
            Reader::Custom(reader) => reader.forward_if(f),
        }
    }

    #[inline]
    fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_while_1(f),
            Reader::Custom(reader) => reader.forward_while_1(f),
        }
    }

    #[inline]
    fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_tag(tag),
            Reader::Custom(reader) => reader.forward_tag(tag),
        }
    }

    #[inline]
    fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        match self {
            Reader::Slice(reader) => reader.forward_while(f),
            Reader::Custom(reader) => reader.forward_while(f),
        }
    }

    #[inline]
    fn peek_tag(&self, tag: &[u8]) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.peek_tag(tag),
            Reader::Custom(reader) => reader.peek_tag(tag),
        }
    }

    #[inline]
    fn read_u16(&mut self) -> Option<u16> {
        match self {
            Reader::Slice(reader) => reader.read_u16(),
            Reader::Custom(reader) => reader.read_u16(),
        }
    }

    #[inline]
    fn read_u32(&mut self) -> Option<u32> {
        match self {
            Reader::Slice(reader) => reader.read_u32(),
            Reader::Custom(reader) => reader.read_u32(),
        }
    }

    #[inline]
    fn read_u64(&mut self) -> Option<u64> {
        match self {
            Reader::Slice(reader) => reader.read_u64(),
            Reader::Custom(reader) => reader.read_u64(),
        }
    }
    
    #[inline]
    fn find_needle(&mut self, needle: &[u8]) -> Option<usize> {
        match self {
            Reader::Slice(reader) => reader.find_needle(needle),
            Reader::Custom(reader) => reader.find_needle(needle),
        }
    }
    
    #[inline]
    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize> {
        match self {
            Reader::Slice(reader) => reader.findr_needle(needle),
            Reader::Custom(reader) => reader.findr_needle(needle),
        }
    }
}

/// A reader for reading bytes and PDF objects.
#[derive(Clone, Debug)]
pub struct SliceReader<'a> {
    /// The underlying data of the reader.
    pub data: ReadBytes<'a>,
    /// The current byte-offset.
    pub offset: usize,
}

impl<'a> SliceReader<'a> {
    /// Create a new reader.
    #[inline]
    pub fn new(data: ReadBytes<'a>) -> Self {
        Self { data, offset: 0 }
    }

    /// Create a new reader at the given offset.
    #[inline]
    pub fn new_with(data: ReadBytes<'a>, offset: usize) -> Self {
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
    fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[inline]
    fn range(&self, range: Range<usize>) -> Option<ReadBytes<'a>> {
        match &self.data.inner() {
            Cow::Borrowed(data) => data.get(range).map(|s| s.into()),
            Cow::Owned(data) => data.get(range).map(|s| s.to_owned().into()),
        }
    }

    #[inline]
    fn offset(&self) -> usize {
        self.offset
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<ReadBytes<'a>> {
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
        let dest = self.offset
            .checked_add(len)
            .filter(|d| *d < self.len())?;
        self.jump(dest);
        Some(())
    }

    #[inline]
    fn peek_bytes(&self, len: usize) -> Option<ReadBytes<'a>> {
        let end = self.offset.checked_add(len)?;
        self.range(self.offset..end)
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
    
    #[inline]
    fn find_needle(&mut self, needle: &[u8]) -> Option<usize> {
        util::find_needle(&self.data[self.offset..], needle)
    }
    
    #[inline]
    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize> {
        util::findr_needle(&self.data[..self.offset], needle)
    }
}

#[derive(Clone, Debug)]
pub struct CustomReader {
    data: Arc<Mutex<dyn CustomSource>>,
    offset: usize,
    len: usize,
}

impl CustomReader {
    fn new(data: Arc<Mutex<dyn CustomSource>>) -> Self {
        let len = data.lock().unwrap().len().unwrap();
        Self {
            data: data,
            offset: 0,
            len: len,
        }
    }
}

impl ByteReader<'static> for CustomReader {
    #[inline]
    fn at_end(&self) -> bool {
        self.offset >= self.len
    }

    #[inline]
    fn jump_to_end(&mut self) {
        self.offset = self.len;
    }

    #[inline]
    fn jump(&mut self, offset: usize) {
        self.offset = offset;
    }

    #[inline]
    fn len(&self) -> usize {
        self.len
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    fn range(&self, range: Range<usize>) -> Option<ReadBytes<'static>> {
        let mut data = self.data.lock().unwrap();
        let read = data.read(range).unwrap()?;
        Some(read.into())
    }

    #[inline]
    fn offset(&self) -> usize {
        self.offset
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<ReadBytes<'static>> {
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
        let dest = self.offset
            .checked_add(len)
            .filter(|d| *d < self.len())?;
        self.jump(dest);
        Some(())
    }

    #[inline]
    fn peek_bytes(&self, len: usize) -> Option<ReadBytes<'static>> {
        let end = self.offset.checked_add(len)?;
        self.range(self.offset..end)
    }

    #[inline]
    fn peek_byte(&self) -> Option<u8> {
        self.data.lock().unwrap().read_byte(self.offset).unwrap()
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
        let mut data = self.data.lock().unwrap();
        while let Some(b) = data.read_byte(self.offset).unwrap() {
            if f(b) {
                self.offset += 1;
            } else {
                break;
            }
        }
    }

    // #[inline]
    // fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
    //     let mut data = self.data.lock().unwrap();
    //     while self.offset < self.len {
    //         let chunk = data.read_next(self.offset).unwrap().unwrap();
    //         for b in chunk {
    //             if f(b) {
    //                 self.offset += 1;
    //             } else {
    //                 return;
    //             }
    //         }
    //     }
    // }

    #[inline]
    fn peek_tag(&self, tag: &[u8]) -> Option<()> {
        let bytes = self.peek_bytes(tag.len())?;
        if tag == bytes.as_ref() {
            Some(())
        } else {
            None
        }
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
        let mut data = self.data.lock().unwrap();
        let mut pos = self.offset;
        let mut chunk = Vec::<u8>::new();
        loop {
            let prev_size = chunk.len();
            let chunk_end = pos.saturating_add(1000).min(self.len);
            chunk.extend(data.read(pos..chunk_end).unwrap()?);
            pos = chunk_end;
            match util::find_needle(chunk.as_slice(), needle) {
                Some(chunk_pos) => return Some(chunk_pos + pos - prev_size),
                None => (),
            }
            chunk = chunk[chunk.len()-needle.len()..].to_vec();
        }
    }
    
    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize> {
        let mut data = self.data.lock().unwrap();
        let mut pos = self.offset;
        let mut prev = Vec::<u8>::new();
        loop {
            let end_pos = pos;
            pos = pos.saturating_sub(1000);
            let mut chunk = data.read(pos..end_pos).unwrap().unwrap();
            chunk.extend(prev);
            match util::findr_needle(chunk.as_slice(), needle) {
                Some(chunk_pos) => return Some(chunk_pos + pos),
                None => (),
            }
            if pos == 0 {
                return None
            }
            prev = chunk[..needle.len().min(chunk.len())].to_vec();
        }
    }
}

fn range_from_bounds<T: RangeBounds<usize>>(bounds: T, len: usize) -> Range<usize> {
    let start = match bounds.start_bound() {
        core::ops::Bound::Included(index) => *index,
        core::ops::Bound::Excluded(index) => *index + 1,
        core::ops::Bound::Unbounded => 0,
    };
    let end = match bounds.end_bound() {
        core::ops::Bound::Included(index) => *index + 1,
        core::ops::Bound::Excluded(index) => *index,
        core::ops::Bound::Unbounded => len,
    };
    start..end
}

pub trait CustomSource: Debug {
    fn len(&mut self) -> std::io::Result<usize>;
    fn read(&mut self, range: Range<usize>) -> std::io::Result<Option<Vec<u8>>>;
    fn read_byte(&mut self, pos: usize) -> std::io::Result<Option<u8>>;
}

#[cfg(test)]
mod tests {
    use super::{ByteReader, SliceReader};

    #[test]
    fn peek_bytes_rejects_overflowing_len() {
        let bytes = b"abc";
        let reader = SliceReader::new(bytes.into());
        assert!(reader.peek_bytes(usize::MAX).is_none());
    }
}
