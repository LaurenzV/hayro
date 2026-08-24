//! A byte reader.

use core::fmt::Debug;
use core::hash::Hash;
use core::ops::Deref;
use core::ops::Range;
use core::ops::RangeBounds;

use alloc::borrow::Cow;

#[cfg(feature = "streaming")]
use crate::data::StreamingSource;
#[cfg(feature = "streaming")]
use crate::sync::Arc;
use crate::util;

/// Struct holding bytes that have been read
#[cfg(not(feature = "streaming"))]
#[derive(Clone, Debug)]
pub struct ReadBytes<'a>(&'a [u8]);

/// Struct holding bytes that have been read
#[cfg(feature = "streaming")]
#[derive(Clone, Debug)]
pub struct ReadBytes<'a>(Cow<'a, [u8]>);

impl<'a> ReadBytes<'a> {
    #[cfg(not(feature = "streaming"))]
    pub const EMPTY: ReadBytes<'static> = ReadBytes(&[]);
    #[cfg(feature = "streaming")]
    pub const EMPTY: ReadBytes<'static> = ReadBytes(Cow::Borrowed(&[]));

    #[cfg(not(feature = "streaming"))]
    #[inline]
    pub fn inner(&self) -> &'a [u8] {
        self.0
    }

    #[cfg(feature = "streaming")]
    #[inline]
    pub fn inner(&self) -> &Cow<'a, [u8]> {
        &self.0
    }

    #[cfg(feature = "streaming")]
    #[inline]
    pub fn into_owned(self) -> Vec<u8> {
        self.0.into_owned()
    }

    #[inline]
    pub fn clone_range<B: RangeBounds<usize>>(&self, range: B) -> Self {
        let range = range_from_bounds(range, self.len());
        #[cfg(not(feature = "streaming"))]
        {
            ReadBytes(&self.0[range])
        }
        #[cfg(feature = "streaming")]
        {
            match &self.0 {
                Cow::Borrowed(data) => ReadBytes(Cow::Borrowed(&data[range])),
                Cow::Owned(data) => ReadBytes(Cow::Owned(data[range].to_vec())),
            }
        }
    }
}

impl<'a> Deref for ReadBytes<'a> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        #[cfg(not(feature = "streaming"))]
        {
            self.0
        }
        #[cfg(feature = "streaming")]
        {
            self.0.deref()
        }
    }
}

impl<'a> PartialEq for ReadBytes<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<'a> Eq for ReadBytes<'a> {}

impl<'a> Hash for ReadBytes<'a> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<'a> AsRef<[u8]> for ReadBytes<'a> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        #[cfg(not(feature = "streaming"))]
        {
            self.0
        }
        #[cfg(feature = "streaming")]
        {
            self.0.as_ref()
        }
    }
}

impl<'a> From<&'a [u8]> for ReadBytes<'a> {
    #[inline]
    fn from(value: &'a [u8]) -> Self {
        #[cfg(not(feature = "streaming"))]
        {
            Self(value)
        }
        #[cfg(feature = "streaming")]
        {
            Self(Cow::Borrowed(value))
        }
    }
}

impl<'a, const C: usize> From<&'a [u8; C]> for ReadBytes<'a> {
    #[inline]
    fn from(value: &'a [u8; C]) -> Self {
        #[cfg(not(feature = "streaming"))]
        {
            Self(value)
        }
        #[cfg(feature = "streaming")]
        {
            Self(Cow::Borrowed(value))
        }
    }
}

#[cfg(feature = "streaming")]
impl<'a> From<Vec<u8>> for ReadBytes<'a> {
    #[inline]
    fn from(value: Vec<u8>) -> Self {
        Self(Cow::Owned(value))
    }
}

impl<'a> From<ReadBytes<'a>> for Cow<'a, [u8]> {
    #[inline]
    fn from(val: ReadBytes<'a>) -> Self {
        #[cfg(not(feature = "streaming"))]
        {
            Cow::Borrowed(val.0)
        }
        #[cfg(feature = "streaming")]
        {
            val.0
        }
    }
}

pub trait ReaderBase<'a> {
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

    /// Returns the index of the first occurrence of the given needle
    fn find_needle(&mut self, needle: &[u8]) -> Option<usize>;

    /// Returns the index of the last occurrence of the given needle
    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize>;
}

#[cfg(not(feature = "streaming"))]
pub type Reader<'a> = SliceReader<'a>;

#[cfg(feature = "streaming")]
#[derive(Clone, Debug)]
pub enum Reader<'a> {
    Slice(SliceReader<'a>),
    Streaming(StreamingReader),
}

#[cfg(feature = "streaming")]
impl<'a> Reader<'a> {
    /// creates reader from byte-slice
    pub fn new(slice: &'a [u8]) -> Self {
        Reader::Slice(SliceReader::new(slice))
    }
    /// creates reader from `ReadBytes`
    pub fn from_read(read: ReadBytes<'a>) -> Self {
        Reader::Slice(SliceReader::from_read(read))
    }
    /// creates reader from `StreamingSource` implementation
    pub fn from_streaming_source(read_seek: Arc<dyn StreamingSource>) -> Self {
        Reader::Streaming(StreamingReader::new(read_seek))
    }
}

#[cfg(feature = "streaming")]
impl<'a> ReaderBase<'a> for Reader<'a> {
    #[inline]
    fn at_end(&self) -> bool {
        match self {
            Reader::Slice(reader) => reader.at_end(),
            Reader::Streaming(reader) => reader.at_end(),
        }
    }

    #[inline]
    fn jump_to_end(&mut self) {
        match self {
            Reader::Slice(reader) => reader.jump_to_end(),
            Reader::Streaming(reader) => reader.jump_to_end(),
        }
    }

    #[inline]
    fn jump(&mut self, offset: usize) {
        match self {
            Reader::Slice(reader) => reader.jump(offset),
            Reader::Streaming(reader) => reader.jump(offset),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            Reader::Slice(reader) => reader.len(),
            Reader::Streaming(reader) => reader.len(),
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        match self {
            Reader::Slice(reader) => reader.is_empty(),
            Reader::Streaming(reader) => reader.is_empty(),
        }
    }

    #[inline]
    fn range(&mut self, range: Range<usize>) -> Option<ReadBytes<'a>> {
        match self {
            Reader::Slice(reader) => reader.range(range),
            Reader::Streaming(reader) => reader.range(range),
        }
    }

    #[inline]
    fn offset(&self) -> usize {
        match self {
            Reader::Slice(reader) => reader.offset(),
            Reader::Streaming(reader) => reader.offset(),
        }
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<ReadBytes<'a>> {
        match self {
            Reader::Slice(reader) => reader.read_bytes(len),
            Reader::Streaming(reader) => reader.read_bytes(len),
        }
    }

    #[inline]
    fn read_byte(&mut self) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.read_byte(),
            Reader::Streaming(reader) => reader.read_byte(),
        }
    }

    #[inline]
    fn skip_bytes(&mut self, len: usize) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.skip_bytes(len),
            Reader::Streaming(reader) => reader.skip_bytes(len),
        }
    }

    #[inline]
    fn peek_bytes(&mut self, len: usize) -> Option<ReadBytes<'_>> {
        match self {
            Reader::Slice(reader) => reader.peek_bytes(len),
            Reader::Streaming(reader) => reader.peek_bytes(len),
        }
    }

    #[inline]
    fn peek_byte(&mut self) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.peek_byte(),
            Reader::Streaming(reader) => reader.peek_byte(),
        }
    }

    #[inline]
    fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        match self {
            Reader::Slice(reader) => reader.eat(f),
            Reader::Streaming(reader) => reader.eat(f),
        }
    }

    #[inline]
    fn forward(&mut self) {
        match self {
            Reader::Slice(reader) => reader.forward(),
            Reader::Streaming(reader) => reader.forward(),
        }
    }

    #[inline]
    fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_if(f),
            Reader::Streaming(reader) => reader.forward_if(f),
        }
    }

    #[inline]
    fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_while_1(f),
            Reader::Streaming(reader) => reader.forward_while_1(f),
        }
    }

    #[inline]
    fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.forward_tag(tag),
            Reader::Streaming(reader) => reader.forward_tag(tag),
        }
    }

    #[inline]
    fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        match self {
            Reader::Slice(reader) => reader.forward_while(f),
            Reader::Streaming(reader) => reader.forward_while(f),
        }
    }

    #[inline]
    fn peek_tag(&mut self, tag: &[u8]) -> Option<()> {
        match self {
            Reader::Slice(reader) => reader.peek_tag(tag),
            Reader::Streaming(reader) => reader.peek_tag(tag),
        }
    }

    #[inline]
    fn read_u16(&mut self) -> Option<u16> {
        match self {
            Reader::Slice(reader) => reader.read_u16(),
            Reader::Streaming(reader) => reader.read_u16(),
        }
    }

    #[inline]
    fn read_u32(&mut self) -> Option<u32> {
        match self {
            Reader::Slice(reader) => reader.read_u32(),
            Reader::Streaming(reader) => reader.read_u32(),
        }
    }

    #[inline]
    fn read_u64(&mut self) -> Option<u64> {
        match self {
            Reader::Slice(reader) => reader.read_u64(),
            Reader::Streaming(reader) => reader.read_u64(),
        }
    }

    #[inline]
    fn find_needle(&mut self, needle: &[u8]) -> Option<usize> {
        match self {
            Reader::Slice(reader) => reader.find_needle(needle),
            Reader::Streaming(reader) => reader.find_needle(needle),
        }
    }

    #[inline]
    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize> {
        match self {
            Reader::Slice(reader) => reader.findr_needle(needle),
            Reader::Streaming(reader) => reader.findr_needle(needle),
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
    /// creates reader from byte-slice
    #[inline]
    pub fn new(slice: &'a [u8]) -> Self {
        SliceReader::from_read(slice.into())
    }

    /// creates reader from `ReadBytes`
    #[inline]
    pub fn from_read(read: ReadBytes<'a>) -> Self {
        Self {
            data: read,
            offset: 0,
        }
    }
}

impl<'a> ReaderBase<'a> for SliceReader<'a> {
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
        #[cfg(not(feature = "streaming"))]
        {
            self.data.inner().get(range).map(move |s| s.into())
        }
        #[cfg(feature = "streaming")]
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
        let dest = self.offset.checked_add(len).filter(|d| *d < self.len())?;
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

// Configuration for StreamingReader

/// size of cache buffer
#[cfg(feature = "streaming")]
const BUFFER_SIZE: usize = 500;

#[derive(Clone, Debug)]
#[cfg(feature = "streaming")]
pub struct StreamingReader {
    data: Arc<dyn StreamingSource>,
    offset: usize,
    len: usize,
    cache: ReaderCache,
}

#[cfg(feature = "streaming")]
impl StreamingReader {
    fn new(data: Arc<dyn StreamingSource>) -> Self {
        let len = data.len().unwrap();
        Self {
            data,
            offset: 0,
            len,
            cache: ReaderCache::default(),
        }
    }

    #[inline]
    fn read_buf(&mut self, range: Range<usize>) -> Option<&[u8]> {
        self.cache.read_buf(range, self.len, |read_range| {
            self.data.read(read_range).unwrap()
        })
    }
}

#[cfg(feature = "streaming")]
impl ReaderBase<'static> for StreamingReader {
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
            Some(self.data.read(range).unwrap()?.into())
        }
    }

    #[inline]
    fn offset(&self) -> usize {
        self.offset
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<ReadBytes<'static>> {
        let read: ReadBytes<'_> = self.peek_bytes(len)?.into_owned().into();
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
        let dest = self.offset.checked_add(len).filter(|d| *d < self.len())?;
        self.jump(dest);
        Some(())
    }

    #[inline]
    fn peek_bytes(&mut self, len: usize) -> Option<ReadBytes<'_>> {
        let end = self.offset.checked_add(len)?;

        let range = self.offset..end;
        if len <= BUFFER_SIZE {
            self.read_buf(range).map(|slice| slice.into())
        } else {
            self.range(range)
        }
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
        while let Some(b) = self.peek_byte() {
            if f(b) {
                self.offset += 1;
            } else {
                break;
            }
        }
    }

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
        let mut pos = self.offset;
        let mut chunk = Vec::<u8>::new();
        loop {
            let prev_size = chunk.len();
            let chunk_end = pos.saturating_add(1000).min(self.len);
            chunk.extend(self.data.read(pos..chunk_end).unwrap()?);
            pos = chunk_end;
            if let Some(chunk_pos) = util::find_needle(chunk.as_slice(), needle) {
                return Some(chunk_pos + pos - prev_size);
            }
            if chunk_end == self.len {
                return None;
            }
            chunk = chunk[chunk.len() - needle.len()..].to_vec();
        }
    }

    fn findr_needle(&mut self, needle: &[u8]) -> Option<usize> {
        let mut pos = self.offset;
        let mut prev = Vec::<u8>::new();
        loop {
            let end_pos = pos;
            pos = pos.saturating_sub(1000);
            let mut chunk = self.data.read(pos..end_pos).unwrap().unwrap();
            chunk.extend(prev);
            if let Some(chunk_pos) = util::findr_needle(chunk.as_slice(), needle) {
                return Some(chunk_pos + pos);
            }
            if pos == 0 {
                return None;
            }
            prev = chunk[..needle.len().min(chunk.len())].to_vec();
        }
    }
}

#[cfg(feature = "streaming")]
#[derive(Clone, Debug, Default)]
pub struct ReaderCache {
    /// buffers for accelerating access
    buffer: CacheBuffer,
}

#[cfg(feature = "streaming")]
#[derive(Clone, Debug)]
struct CacheBuffer {
    /// range of this buffer is valid for
    range: Range<usize>,
    /// actual buffered data
    data: Vec<u8>,
}

#[cfg(feature = "streaming")]
impl Default for CacheBuffer {
    fn default() -> Self {
        Self {
            range: 0..0,
            data: Vec::new(),
        }
    }
}

#[cfg(feature = "streaming")]
impl ReaderCache {
    /// Reads buffer using cache
    /// * `range` - The range to read
    /// * `data_len` - The length of the data-source to read from
    /// * `read_data` - Callback to actually read data from source
    #[inline]
    fn read_buf(
        &mut self,
        range: Range<usize>,
        data_len: usize,
        read_data: impl FnOnce(Range<usize>) -> Option<Vec<u8>>,
    ) -> Option<&[u8]> {
        if range.end > data_len {
            return None;
        }

        let buffer = &mut self.buffer;
        let buf_start = buffer.range.start;
        let buf_end = buffer.range.end;
        if range.start >= buf_start && range.end <= buf_end {
            Some(&buffer.data[range.start - buf_start..range.end - buf_start])
        } else {
            let new_start = range.start;
            let new_end = (new_start + BUFFER_SIZE).min(data_len);
            let new_range = new_start..new_end;
            let bytes = read_data(new_range.clone())?;
            *buffer = CacheBuffer {
                range: new_range,
                data: bytes,
            };
            Some(&buffer.data[0..(range.end - range.start)])
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

#[cfg(test)]
mod tests {
    use super::{ReaderBase, SliceReader};

    #[test]
    fn peek_bytes_rejects_overflowing_len() {
        let bytes = b"abc";
        let mut reader = SliceReader::new(bytes);
        assert!(reader.peek_bytes(usize::MAX).is_none());
    }
}
