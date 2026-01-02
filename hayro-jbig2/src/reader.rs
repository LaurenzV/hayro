//! Combined byte and bit reader utilities.

/// A reader for reading bytes and bits from a slice.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    cur_pos: usize,
}

impl<'a> Reader<'a> {
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

    #[inline]
    pub(crate) fn at_end(&self) -> bool {
        self.byte_pos() >= self.data.len()
    }

    #[inline]
    pub(crate) fn tail(&self) -> Option<&'a [u8]> {
        self.data.get(self.byte_pos()..)
    }

    #[inline]
    pub(crate) fn offset(&self) -> usize {
        self.byte_pos()
    }

    #[inline]
    pub(crate) fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        debug_assert_eq!(self.bit_pos(), 0);

        let bytes = self.peek_bytes(len)?;
        self.cur_pos += len * 8;

        Some(bytes)
    }

    #[inline]
    pub(crate) fn read_byte(&mut self) -> Option<u8> {
        debug_assert_eq!(self.bit_pos(), 0);

        let byte = self.peek_byte()?;
        self.cur_pos += 8;

        Some(byte)
    }

    #[inline]
    pub(crate) fn skip_bytes(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| ())
    }

    #[inline]
    pub(crate) fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        let start = self.byte_pos();
        let end = start.checked_add(len)?;
        self.data.get(start..end)
    }

    #[inline]
    pub(crate) fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.byte_pos()).copied()
    }

    #[inline]
    pub(crate) fn read_u16(&mut self) -> Option<u16> {
        let bytes = self.read_bytes(2)?;

        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    #[inline]
    pub(crate) fn read_u32(&mut self) -> Option<u32> {
        let bytes = self.read_bytes(4)?;

        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    #[inline]
    pub(crate) fn read_i32(&mut self) -> Option<i32> {
        let bytes = self.read_bytes(4)?;

        Some(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    #[inline(always)]
    pub(crate) fn read_bit(&mut self) -> Option<u32> {
        let byte_pos = self.byte_pos();
        let byte = *self.data.get(byte_pos)? as u32;
        let shift = 7 - self.bit_pos();
        self.cur_pos += 1;
        Some((byte >> shift) & 1)
    }

    #[inline(always)]
    pub(crate) fn read_bits(&mut self, count: u8) -> Result<u32, &'static str> {
        let mut value = 0_u32;
        for _ in 0..count {
            let bit = self
                .read_bit()
                .ok_or("unexpected end of data reading bits")?;
            value = (value << 1) | bit;
        }
        Ok(value)
    }

    #[inline]
    pub(crate) fn byte_pos(&self) -> usize {
        self.cur_pos / 8
    }

    #[inline]
    pub(crate) fn bit_pos(&self) -> usize {
        self.cur_pos % 8
    }
}
