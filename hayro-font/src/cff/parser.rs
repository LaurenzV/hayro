//! Binary parsing utils.
//!
//! This module should not be used directly, unless you're planning to parse
//! some tables manually.

use crate::GlyphId;
use core::convert::TryInto;

/// A trait for parsing raw binary data of fixed size.
///
/// This is a low-level, internal trait that should not be used directly.
pub(crate) trait FromData: Sized {
    /// Object's raw data size.
    ///
    /// Not always the same as `mem::size_of`.
    const SIZE: usize;

    /// Parses an object from a raw data.
    fn parse(data: &[u8]) -> Option<Self>;
}

/// A trait for parsing raw binary data of variable size.
///
/// This is a low-level, internal trait that should not be used directly.
pub(crate) trait FromSlice<'a>: Sized {
    /// Parses an object from a raw data.
    fn parse(data: &'a [u8]) -> Option<Self>;
}

impl FromData for () {
    const SIZE: usize = 0;

    #[inline]
    fn parse(_: &[u8]) -> Option<Self> {
        Some(())
    }
}

impl FromData for u8 {
    const SIZE: usize = 1;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        data.first().copied()
    }
}

impl FromData for i8 {
    const SIZE: usize = 1;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        data.first().copied().map(|n| n as Self)
    }
}

impl FromData for u16 {
    const SIZE: usize = 2;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        data.try_into().ok().map(Self::from_be_bytes)
    }
}

impl FromData for i16 {
    const SIZE: usize = 2;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        data.try_into().ok().map(Self::from_be_bytes)
    }
}

impl FromData for u32 {
    const SIZE: usize = 4;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        data.try_into().ok().map(Self::from_be_bytes)
    }
}

impl FromData for i32 {
    const SIZE: usize = 4;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        data.try_into().ok().map(Self::from_be_bytes)
    }
}

impl FromData for u64 {
    const SIZE: usize = 8;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        data.try_into().ok().map(Self::from_be_bytes)
    }
}

/// A u24 number.
///
/// Stored as u32, but encoded as 3 bytes in the font.
///
/// <https://docs.microsoft.com/en-us/typography/opentype/spec/otff#data-types>
#[derive(Clone, Copy, Debug)]
pub(crate) struct U24(pub(crate) u32);

impl FromData for U24 {
    const SIZE: usize = 3;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        let data: [u8; 3] = data.try_into().ok()?;
        Some(Self(u32::from_be_bytes([0, data[0], data[1], data[2]])))
    }
}

/// A 32-bit signed fixed-point number (16.16).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fixed(pub(crate) f32);

impl FromData for Fixed {
    const SIZE: usize = 4;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        // TODO: is it safe to cast?
        i32::parse(data).map(|n| Self(n as f32 / 65536.0))
    }
}

/// A safe u32 to usize casting.
///
/// Rust doesn't implement `From<u32> for usize`,
/// because it has to support 16 bit targets.
/// We don't, so we can allow this.
pub(crate) trait NumFrom<T>: Sized {
    /// Converts u32 into usize.
    fn num_from(_: T) -> Self;
}

impl NumFrom<u32> for usize {
    #[inline]
    fn num_from(v: u32) -> Self {
        #[cfg(any(target_pointer_width = "32", target_pointer_width = "64"))]
        {
            v as Self
        }

        // compilation error on 16 bit targets
    }
}

impl NumFrom<char> for usize {
    #[inline]
    fn num_from(v: char) -> Self {
        #[cfg(any(target_pointer_width = "32", target_pointer_width = "64"))]
        {
            v as Self
        }

        // compilation error on 16 bit targets
    }
}

/// A slice-like container that converts internal binary data only on access.
///
/// Array values are stored in a continuous data chunk.
#[derive(Clone, Copy)]
pub(crate) struct LazyArray16<'a, T> {
    data: &'a [u8],
    data_type: core::marker::PhantomData<T>,
}

impl<T> Default for LazyArray16<'_, T> {
    #[inline]
    fn default() -> Self {
        LazyArray16 {
            data: &[],
            data_type: core::marker::PhantomData,
        }
    }
}

impl<'a, T: FromData> LazyArray16<'a, T> {
    /// Creates a new `LazyArray`.
    #[inline]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        LazyArray16 {
            data,
            data_type: core::marker::PhantomData,
        }
    }

    /// Returns a value at `index`.
    #[inline]
    pub(crate) fn get(&self, index: u16) -> Option<T> {
        if index < self.len() {
            let start = usize::from(index) * T::SIZE;
            let end = start + T::SIZE;
            self.data.get(start..end).and_then(T::parse)
        } else {
            None
        }
    }

    /// Returns array's length.
    #[inline]
    pub(crate) fn len(&self) -> u16 {
        (self.data.len() / T::SIZE) as u16
    }

    /// Checks if array is empty.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a, T: FromData + core::fmt::Debug + Copy> core::fmt::Debug for LazyArray16<'a, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(*self).finish()
    }
}

impl<'a, T: FromData> IntoIterator for LazyArray16<'a, T> {
    type Item = T;
    type IntoIter = LazyArrayIter16<'a, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        LazyArrayIter16 {
            data: self,
            index: 0,
        }
    }
}

/// An iterator over `LazyArray16`.
#[derive(Clone, Copy)]
#[allow(missing_debug_implementations)]
pub(crate) struct LazyArrayIter16<'a, T> {
    data: LazyArray16<'a, T>,
    index: u16,
}

impl<T: FromData> Default for LazyArrayIter16<'_, T> {
    #[inline]
    fn default() -> Self {
        LazyArrayIter16 {
            data: LazyArray16::new(&[]),
            index: 0,
        }
    }
}

impl<'a, T: FromData> Iterator for LazyArrayIter16<'a, T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.index += 1; // TODO: check
        self.data.get(self.index - 1)
    }

    #[inline]
    fn count(self) -> usize {
        usize::from(self.data.len().saturating_sub(self.index))
    }
}

/// A [`LazyArray16`]-like container, but data is accessed by offsets.
///
/// Unlike [`LazyArray16`], internal storage is not continuous.
///
/// Multiple offsets can point to the same data.
#[derive(Clone, Copy)]
pub(crate) struct LazyOffsetArray16<'a, T: FromSlice<'a>> {
    data: &'a [u8],
    // Zero offsets must be ignored, therefore we're using `Option<Offset16>`.
    offsets: LazyArray16<'a, Option<Offset16>>,
    data_type: core::marker::PhantomData<T>,
}

impl<'a, T: FromSlice<'a>> LazyOffsetArray16<'a, T> {
    /// Creates a new `LazyOffsetArray16`.
    #[allow(dead_code)]
    pub(crate) fn new(data: &'a [u8], offsets: LazyArray16<'a, Option<Offset16>>) -> Self {
        Self {
            data,
            offsets,
            data_type: core::marker::PhantomData,
        }
    }

    /// Parses `LazyOffsetArray16` from raw data.
    #[allow(dead_code)]
    pub(crate) fn parse(data: &'a [u8]) -> Option<Self> {
        let mut s = Stream::new(data);
        let count = s.read::<u16>()?;
        let offsets = s.read_array16(count)?;
        Some(Self {
            data,
            offsets,
            data_type: core::marker::PhantomData,
        })
    }

    /// Returns a value at `index`.
    #[inline]
    pub(crate) fn get(&self, index: u16) -> Option<T> {
        let offset = self.offsets.get(index)??.to_usize();
        self.data.get(offset..).and_then(T::parse)
    }

    /// Returns array's length.
    #[inline]
    pub(crate) fn len(&self) -> u16 {
        self.offsets.len()
    }

    /// Checks if array is empty.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a, T: FromSlice<'a> + core::fmt::Debug + Copy> core::fmt::Debug for LazyOffsetArray16<'a, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(*self).finish()
    }
}

impl FromData for GlyphId {
    const SIZE: usize = 2;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        u16::parse(data).map(GlyphId)
    }
}

/// An iterator over [`LazyOffsetArray16`] values.
#[derive(Clone, Copy)]
#[allow(missing_debug_implementations)]
pub(crate) struct LazyOffsetArrayIter16<'a, T: FromSlice<'a>> {
    array: LazyOffsetArray16<'a, T>,
    index: u16,
}

impl<'a, T: FromSlice<'a>> IntoIterator for LazyOffsetArray16<'a, T> {
    type Item = T;
    type IntoIter = LazyOffsetArrayIter16<'a, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        LazyOffsetArrayIter16 {
            array: self,
            index: 0,
        }
    }
}

impl<'a, T: FromSlice<'a>> Iterator for LazyOffsetArrayIter16<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.array.len() {
            self.index += 1;
            self.array.get(self.index - 1)
        } else {
            None
        }
    }

    #[inline]
    fn count(self) -> usize {
        usize::from(self.array.len().saturating_sub(self.index))
    }
}

/// A streaming binary parser.
#[derive(Clone, Default, Debug)]
pub(crate) struct Stream<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Stream<'a> {
    /// Creates a new `Stream` parser.
    #[inline]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Stream { data, offset: 0 }
    }

    /// Creates a new `Stream` parser at offset.
    ///
    /// Returns `None` when `offset` is out of bounds.
    #[inline]
    pub(crate) fn new_at(data: &'a [u8], offset: usize) -> Option<Self> {
        if offset <= data.len() {
            Some(Stream { data, offset })
        } else {
            None
        }
    }

    /// Checks that stream reached the end of the data.
    #[inline]
    pub(crate) fn at_end(&self) -> bool {
        self.offset >= self.data.len()
    }

    /// Returns the current offset.
    #[inline]
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the trailing data.
    ///
    /// Returns `None` when `Stream` is reached the end.
    #[inline]
    pub(crate) fn tail(&self) -> Option<&'a [u8]> {
        self.data.get(self.offset..)
    }

    /// Advances by `FromData::SIZE`.
    ///
    /// Doesn't check bounds.
    #[inline]
    pub(crate) fn skip<T: FromData>(&mut self) {
        self.advance(T::SIZE);
    }

    /// Advances by the specified `len`.
    ///
    /// Doesn't check bounds.
    #[inline]
    pub(crate) fn advance(&mut self, len: usize) {
        self.offset += len;
    }

    /// Parses the type from the steam.
    ///
    /// Returns `None` when there is not enough data left in the stream
    /// or the type parsing failed.
    #[inline]
    pub(crate) fn read<T: FromData>(&mut self) -> Option<T> {
        self.read_bytes(T::SIZE).and_then(T::parse)
    }

    /// Reads N bytes from the stream.
    #[inline]
    pub(crate) fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        // An integer overflow here on 32bit systems is almost guarantee to be caused
        // by an incorrect parsing logic from the caller side.
        // Simply using `checked_add` here would silently swallow errors, which is not what we want.
        debug_assert!(self.offset as u64 + len as u64 <= u32::MAX as u64);

        let v = self.data.get(self.offset..self.offset + len)?;
        self.advance(len);
        Some(v)
    }

    /// Reads the next `count` types as a slice.
    #[inline]
    pub(crate) fn read_array16<T: FromData>(&mut self, count: u16) -> Option<LazyArray16<'a, T>> {
        let len = usize::from(count) * T::SIZE;
        self.read_bytes(len).map(LazyArray16::new)
    }

    #[allow(dead_code)]
    #[inline]
    pub(crate) fn read_at_offset16(&mut self, data: &'a [u8]) -> Option<&'a [u8]> {
        let offset = self.read::<Offset16>()?.to_usize();
        data.get(offset..)
    }
}

/// A common offset methods.
pub(crate) trait Offset {
    /// Converts the offset to `usize`.
    fn to_usize(&self) -> usize;
}

/// A type-safe u16 offset.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Offset16(pub(crate) u16);

impl Offset for Offset16 {
    #[inline]
    fn to_usize(&self) -> usize {
        usize::from(self.0)
    }
}

impl FromData for Offset16 {
    const SIZE: usize = 2;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        u16::parse(data).map(Offset16)
    }
}

impl FromData for Option<Offset16> {
    const SIZE: usize = Offset16::SIZE;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        let offset = Offset16::parse(data)?;
        if offset.0 != 0 {
            Some(Some(offset))
        } else {
            Some(None)
        }
    }
}

/// A type-safe u24 offset.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Offset24(pub(crate) u32);

impl Offset for Offset24 {
    #[inline]
    fn to_usize(&self) -> usize {
        usize::num_from(self.0)
    }
}

impl FromData for Offset24 {
    const SIZE: usize = 3;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        U24::parse(data).map(|n| Self(n.0))
    }
}

impl FromData for Option<Offset24> {
    const SIZE: usize = Offset24::SIZE;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        let offset = Offset24::parse(data)?;
        if offset.0 != 0 {
            Some(Some(offset))
        } else {
            Some(None)
        }
    }
}

/// A type-safe u32 offset.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Offset32(pub(crate) u32);

impl Offset for Offset32 {
    #[inline]
    fn to_usize(&self) -> usize {
        usize::num_from(self.0)
    }
}

impl FromData for Offset32 {
    const SIZE: usize = 4;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        u32::parse(data).map(Offset32)
    }
}

impl FromData for Option<Offset32> {
    const SIZE: usize = Offset32::SIZE;

    #[inline]
    fn parse(data: &[u8]) -> Option<Self> {
        let offset = Offset32::parse(data)?;
        if offset.0 != 0 {
            Some(Some(offset))
        } else {
            Some(None)
        }
    }
}
