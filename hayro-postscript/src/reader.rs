use core::ops::Range;

#[derive(Clone, Debug)]
pub(crate) struct Reader<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) offset: usize,
}

impl<'a> Reader<'a> {
    #[inline]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    #[inline]
    pub(crate) fn jump(&mut self, offset: usize) {
        self.offset = offset;
    }

    #[inline]
    pub(crate) fn range(&self, range: Range<usize>) -> Option<&'a [u8]> {
        self.data.get(range)
    }

    #[inline]
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    #[inline]
    pub(crate) fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let v = self.peek_bytes(len)?;
        self.offset += len;
        Some(v)
    }

    #[inline]
    pub(crate) fn read_byte(&mut self) -> Option<u8> {
        let v = self.peek_byte()?;
        self.offset += 1;
        Some(v)
    }

    #[inline]
    pub(crate) fn peek_bytes(&self, len: usize) -> Option<&'a [u8]> {
        self.data.get(self.offset..self.offset + len)
    }

    #[inline]
    pub(crate) fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.offset).copied()
    }

    #[inline]
    pub(crate) fn eat(&mut self, f: impl Fn(u8) -> bool) -> Option<u8> {
        let val = self.peek_byte()?;
        if f(val) {
            self.forward();
            Some(val)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn forward(&mut self) {
        self.offset += 1;
    }

    #[inline]
    pub(crate) fn forward_while(&mut self, f: impl Fn(u8) -> bool) {
        while let Some(b) = self.peek_byte() {
            if f(b) {
                self.forward();
            } else {
                break;
            }
        }
    }

    #[inline]
    pub(crate) fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        self.peek_tag(tag)?;
        self.offset += tag.len();
        Some(())
    }

    #[inline]
    pub(crate) fn peek_tag(&self, tag: &[u8]) -> Option<()> {
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
    pub(crate) fn skip_eol(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_eol(b) {
                self.forward();
            } else {
                return;
            }
        }
    }
}

#[inline(always)]
pub(crate) fn is_whitespace(b: u8) -> bool {
    matches!(b, 0x00 | 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}

#[inline(always)]
pub(crate) fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

#[inline(always)]
pub(crate) fn is_regular(b: u8) -> bool {
    !is_whitespace(b) && !is_delimiter(b)
}

#[inline(always)]
pub(crate) fn is_eol(b: u8) -> bool {
    matches!(b, 0x0a | 0x0d)
}
