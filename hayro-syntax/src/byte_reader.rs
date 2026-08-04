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
    pub fn into_owned(self) -> Vec<u8> {
        self.0.into_owned()
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
    fn range(&mut self, range: Range<usize>) -> Option<ReadBytes<'a>>;

    /// Returns the current offset of the reader.
    fn offset(&self) -> usize;

    /// Reads the specified number of bytes and advances the offset.
    fn read_bytes(&mut self, len: usize) -> Option<ReadBytes<'a>>;

    /// Reads a single byte and advances the offset.
    fn read_byte(&mut self) -> Option<u8>;

    /// Skips the specified number of bytes by advancing the offset.
    fn skip_bytes(&mut self, len: usize) -> Option<()>;

    /// Peeks the specified number of bytes.
    fn peek_bytes(&mut self, len: usize) -> Option<ReadBytes<'_>>;

    /// Peeks a single byte.
    fn peek_byte(&mut self) -> Option<u8>;

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
    fn peek_tag(&mut self, tag: &[u8]) -> Option<()>;

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
pub enum Reader<'a, 'c> {
    Slice(SliceReader<'a>),
    Custom(CustomReader<'c>),
}

impl <'a, 'c> Reader<'a, 'c> {
    pub fn from_read(read: ReadBytes<'a>) -> Self {
        Reader::Slice(SliceReader::new(read))
    }
    pub fn from_slice(slice: &'a [u8]) -> Self {
        Reader::Slice(SliceReader::new(slice.into()))
    }
    pub fn from_custom_source(read_seek: Arc<Mutex<dyn CustomSource>>) -> Self {
        Reader::Custom(CustomReader::new(read_seek))
    }
    pub fn from_custom_source_with_cache(read_seek: Arc<Mutex<dyn CustomSource>>, cache: &'c mut ReaderCache) -> Self {
        Reader::Custom(CustomReader::new_with_cache(read_seek, cache))
    }
}

impl<'a> ByteReader<'a> for Reader<'a, '_> {
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
    fn range(&mut self, range: Range<usize>) -> Option<ReadBytes<'a>> {
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
    fn peek_bytes(&mut self, len: usize) -> Option<ReadBytes<'_>> {
        match self {
            Reader::Slice(reader) => reader.peek_bytes(len),
            Reader::Custom(reader) => reader.peek_bytes(len),
        }
    }

    #[inline]
    fn peek_byte(&mut self) -> Option<u8> {
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
    fn peek_tag(&mut self, tag: &[u8]) -> Option<()> {
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
    fn range(&mut self, range: Range<usize>) -> Option<ReadBytes<'a>> {
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
        let end = self.offset.checked_add(len)?;
        let v = self.range(self.offset..end)?;
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
    fn peek_bytes(&mut self, len: usize) -> Option<ReadBytes<'_>> {
        let end = self.offset.checked_add(len)?;
        self.range(self.offset..end)
    }

    #[inline]
    fn peek_byte(&mut self) -> Option<u8> {
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
    fn peek_tag(&mut self, tag: &[u8]) -> Option<()> {
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
pub struct CustomReader<'c> {
    data: Arc<Mutex<dyn CustomSource>>,
    offset: usize,
    len: usize,
    cache: CacheInstance<'c>,
}

impl CustomReader<'static> {
    fn new(data: Arc<Mutex<dyn CustomSource>>) -> Self {
        let len = data.lock().unwrap().len().unwrap();
        Self {
            data: data,
            offset: 0,
            len: len,
            cache: ReaderCache::default().into(),
        }
    }
}

impl<'c> CustomReader<'c> {
    fn new_with_cache(data: Arc<Mutex<dyn CustomSource>>, cache: &'c mut ReaderCache) -> Self {
        let len = data.lock().unwrap().len().unwrap();
        Self {
            data: data,
            offset: 0,
            len: len,
            cache: cache.into(),
        }
    }
}

impl<'c> CustomReader<'c> {
    #[inline]
    fn read_buf(&mut self, range: Range<usize>) -> Option<&[u8]> {
        self.cache.as_mut().read_buf(range, self.len, |read_range| {
            let mut data = self.data.lock().unwrap();
            data.read(read_range).unwrap()
        })
    }
}

impl ByteReader<'static> for CustomReader<'_> {
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
    fn range(&mut self, range: Range<usize>) -> Option<ReadBytes<'static>> {
        let len = range.end - range.start;
        if len <= BUFFER_SIZE {
            self.read_buf(range).map(|r| r.to_vec().into())
        } else {
            let mut data = self.data.lock().unwrap();
            Some(data.read(range).unwrap()?.into())
        }
    }

    #[inline]
    fn offset(&self) -> usize {
        self.offset
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<ReadBytes<'static>> {
        let read = self.peek_bytes(len)?.into_owned().into();
        self.offset += len;
        Some(read)
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
    fn peek_bytes(&mut self, len: usize) -> Option<ReadBytes<'_>> {
        let end = self.offset.checked_add(len)?;
        self.range(self.offset..end)
    }

    #[inline]
    fn peek_byte(&mut self) -> Option<u8> {
        self.peek_bytes(1).map(|b| b.as_ref()[0])
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
        // let mut data = self.data.lock().unwrap();
        while let Some(b) = self.peek_byte() {
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
    fn peek_tag(&mut self, tag: &[u8]) -> Option<()> {
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


#[derive(Debug)]
enum CacheInstance<'c> {
    Owned(ReaderCache),
    Borrowed(&'c mut ReaderCache)
}

impl From<ReaderCache> for CacheInstance<'static> {
    fn from(value: ReaderCache) -> Self {
        Self::Owned(value)
    }
}
impl<'c> From<&'c mut ReaderCache> for CacheInstance<'c> {
    fn from(value: &'c mut ReaderCache) -> Self {
        Self::Borrowed(value)
    }
}

impl AsRef<ReaderCache> for CacheInstance<'_> {
    fn as_ref(&self) -> &ReaderCache {
        match self {
            CacheInstance::Owned(reader_cache) => reader_cache,
            CacheInstance::Borrowed(reader_cache) => reader_cache,
        }
    }
}

impl AsMut<ReaderCache> for CacheInstance<'_> {
    fn as_mut<'s>(&'s mut self) -> &'s mut ReaderCache {
        match self {
            CacheInstance::Owned(reader_cache) => reader_cache,
            CacheInstance::Borrowed(reader_cache) => reader_cache,
        }
    }
}

impl Clone for CacheInstance<'_> {
    fn clone(&self) -> Self {
        CacheInstance::Owned(self.as_ref().clone())
    }
}

/// count of buffers
const BUFFER_COUNT: usize = 3;
/// size of buffers
const BUFFER_SIZE: usize = 500;

#[derive(Clone, Debug, Default)]
pub struct ReaderCache {
    /// buffers for accelerating access
    buffers: [CacheBuffer; BUFFER_COUNT],
    /// LRU counter to fill `CacheBuffer::used` with upon use
    lru_counter: u64,
}

#[derive(Clone, Debug)]
struct CacheBuffer {
    /// range of this buffer is valid for
    range: Range<usize>,
    /// actual buffered data
    data: Vec<u8>,
    /// LRU index, the higher the more recently used
    used: u64,
}

impl Default for CacheBuffer {
    fn default() -> Self {
        Self {
            range: 0..0,
            data: Vec::new(),
            used: 0,
        }
    }
}

impl ReaderCache {
    /// Reads buffer using cache
    /// * `range` - The range to read
    /// * `data_len` - The length of the data-source to read from
    /// * `read_data` - Callback to actually read data from source
    #[inline]
    fn read_buf(&mut self, range: Range<usize>, data_len: usize, read_data: impl FnOnce(Range<usize>) -> Option<Vec<u8>>) -> Option<&[u8]> {
        if range.end > data_len {
            return None;
        }

        let found_hit = self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            let buf_start = buffer.range.start;
            let buf_end = buffer.range.end;
            if range.start >= buf_start && range.end <= buf_end {
                Some((index, range.start-buf_start..range.end-buf_start))
            } else {
                None
            }
        });

        if let Some((index,  range)) = found_hit {
            let buffer = &mut self.buffers[index];
            buffer.used = self.lru_counter;
            self.lru_counter += 1;
            Some(&buffer.data[range])
        } else {
            self.handle_cache_miss(range, data_len, read_data)
        }
    }

    fn handle_cache_miss(&mut self, range: Range<usize>, data_len: usize, read_data: impl FnOnce(Range<usize>) -> Option<Vec<u8>>) -> Option<&[u8]> {
        // println!("cache miss {range:?} (len {})", range.end - range.start);
        let new_start = range.start;
        let new_end = (new_start + BUFFER_SIZE).min(data_len);
        let new_range = new_start..new_end;
        let (cached_start, inner_range, cached_end) = self.get_overlap(new_range.clone());
        let bytes = if inner_range.is_empty() {
            let mut bytes = Vec::with_capacity(range.len());
            bytes.extend_from_slice(cached_start);
            bytes.extend_from_slice(cached_end);
            bytes
        } else {
            let read_bytes = read_data(inner_range)?;
            if !cached_start.is_empty() || !cached_end.is_empty() {
                let mut bytes = Vec::with_capacity(range.len());
                bytes.extend_from_slice(cached_start);
                bytes.extend(read_bytes);
                bytes.extend_from_slice(cached_end);
                bytes
            } else {
                read_bytes
            }
        };
        let buffer = self.buffers.iter_mut().min_by(|a, b| a.used.cmp(&b.used)).unwrap();
        *buffer = CacheBuffer { range: new_range, data: bytes, used: self.lru_counter };
        self.lru_counter += 1;
        Some(&buffer.data[0..(range.end - range.start)])
    }

    #[inline]
    fn get_overlap(&self, range: Range<usize>) -> (&[u8], Range<usize>, &[u8]) {
        // (&[], range, &[])
        let start = self.buffers.iter()
            .filter(|b| !range.contains(&b.range.start) && range.contains(&b.range.end))
            .max_by(|a, b| a.range.end.cmp(&b.range.end))
            .map(|b| &b.data[(range.start - b.range.start)..])
            .unwrap_or_default();
        // let mut end: &[u8] = &[];
        let mut end = self.buffers.iter()
            .filter(|b| range.contains(&b.range.start) && !range.contains(&b.range.end))
            .min_by(|a, b| a.range.start.cmp(&b.range.start))
            .map(|b| &b.data[..(b.data.len() - (b.range.end - range.end))])
            .unwrap_or_default();

        if start.len() + end.len() > range.len() {
            // start and end are overlapping, truncate end
            let overlap = start.len() + end.len() - range.len();
            end = &end[overlap..];
        }

        let inner_range = (range.start + start.len())..(range.end - end.len());
        (start, inner_range, end)
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
}

#[cfg(test)]
mod tests {
    use super::{ByteReader, SliceReader};

    #[test]
    fn peek_bytes_rejects_overflowing_len() {
        let bytes = b"abc";
        let mut reader = SliceReader::new(bytes.into());
        assert!(reader.peek_bytes(usize::MAX).is_none());
    }
}
