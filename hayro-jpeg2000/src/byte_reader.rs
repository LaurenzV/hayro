//! A byte reader.

/// A reader for reading bytes and PDF objects.
#[derive(Clone, Debug)]
pub(crate) struct Reader<'a> {
    /// The underlying data of the reader.
    pub(crate) data: &'a [u8],
    /// The current byte-offset.
    pub(crate) offset: usize,
}

impl<'a> Reader<'a> {
    /// Create a new reader.
    #[inline]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    /// Returns `true` if the reader has reached the end of the data.
    #[inline]
    pub(crate) fn at_end(&self) -> bool {
        self.offset >= self.data.len()
    }

    /// Moves the reader offset to the end of the data.
    #[inline]
    pub(crate) fn jump_to_end(&mut self) {
        self.offset = self.data.len();
    }

    /// Returns the remaining data from the current offset to the end.
    #[inline]
    pub(crate) fn tail(&mut self) -> Option<&'a [u8]> {
        self.data.get(self.offset..)
    }

    /// Returns the current offset of the reader.
    #[inline]
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Reads the specified number of bytes and advances the offset.
    #[inline]
    pub(crate) fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let v = self.peek_bytes(len)?;
        self.offset += len;

        Some(v)
    }

    /// Reads a single byte and advances the offset.
    #[inline]
    pub(crate) fn read_byte(&mut self) -> Option<u8> {
        let v = self.peek_byte()?;
        self.offset += 1;

        Some(v)
    }

    /// Skips the specified number of bytes by advancing the offset.
    #[inline]
    pub(crate) fn skip_bytes(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| {})
    }

    /// Peeks the specified number of bytes.
    #[inline]
    pub(crate) fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        self.data.get(self.offset..self.offset + len)
    }

    /// Peeks a single byte.
    #[inline]
    pub(crate) fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.offset).copied()
    }

    /// Read a u16 integer (in big endian order).
    #[inline]
    pub(crate) fn read_u16(&mut self) -> Option<u16> {
        let bytes = self.read_bytes(2)?;

        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Read a u32 integer (in big endian order).
    #[inline]
    pub(crate) fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.read_bytes(4)?;

        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a u64 integer (in big endian order).
    #[inline]
    pub(crate) fn read_u64(&mut self) -> Option<u64> {
        let bytes = self.read_bytes(8)?;

        Some(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
}
