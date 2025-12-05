/// A streaming binary parser.
#[derive(Clone, Default, Debug)]
pub(crate) struct Stream<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Stream<'a> {
    #[inline]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Stream { data, offset: 0 }
    }

    #[inline]
    pub(crate) fn read_byte(&mut self) -> Option<u8> {
        let b = self.data.get(self.offset)?;
        self.advance(1);
        Some(*b)
    }

    #[inline]
    pub(crate) fn at_end(&self) -> bool {
        self.offset >= self.data.len()
    }

    #[inline]
    pub(crate) fn peek_byte(&mut self) -> Option<u8> {
        self.clone().read_byte()
    }

    #[inline]
    pub(crate) fn tail(&self) -> Option<&'a [u8]> {
        self.data.get(self.offset..)
    }

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

    #[inline]
    pub(crate) fn advance(&mut self, len: usize) {
        self.offset += len;
    }

    pub(crate) fn move_back(&mut self, amount: usize) {
        self.offset -= amount;
    }
}
