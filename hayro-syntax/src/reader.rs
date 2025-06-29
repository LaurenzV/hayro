//! Reading bytes and PDF objects from data.

use crate::object::ObjectIdentifier;
use crate::trivia::{Comment, is_eol_character, is_white_space_character};
use crate::xref::XRef;
use std::ops::Range;

/// A reader for reading bytes and PDF objects.
#[derive(Clone, Debug)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    #[inline]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
    #[inline]
    pub(crate) fn new_with(data: &'a [u8], offset: usize) -> Self {
        Self { data, offset }
    }

    #[inline]
    pub(crate) fn at_end(&self) -> bool {
        self.offset >= self.data.len()
    }

    #[inline]
    pub(crate) fn jump_to_end(&mut self) {
        self.offset = self.data.len();
    }

    #[inline]
    pub(crate) fn jump(&mut self, offset: usize) {
        self.offset = offset;
    }

    pub(crate) fn head(&self) -> Option<&'a [u8]> {
        self.data.get(..self.offset)
    }

    #[inline]
    pub(crate) fn tail(&mut self) -> Option<&'a [u8]> {
        self.data.get(self.offset..)
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.data.len()
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

    // Note: If `PLAIN` is true, it means that the data we are about to read _might_ contain
    // an object reference instead of an actual object. if `PLAIN` is false, then an object
    // reference cannot occur. The main reason we make this distinction is that when parsing
    // a number, we cannot unambiguously distinguish whether it's a real number or the
    // start of an object reference. In content streams, object references cannot appear,
    // so in order to speed this up we set `PLAIN` to false, meaning that as soon as we
    // encounter a number we know it's a number, and don't need to do a look-ahead to ensure
    // that it's not an object reference.
    #[inline]
    pub(crate) fn read<T: Readable<'a>>(&mut self, ctx: ReaderContext<'a>) -> Option<T> {
        let old_offset = self.offset;

        T::read(self, ctx).or_else(|| {
            self.offset = old_offset;

            None
        })
    }

    #[inline]
    pub(crate) fn read_with_context<T: Readable<'a>>(
        &mut self,
        ctx: ReaderContext<'a>,
    ) -> Option<T> {
        self.read::<T>(ctx)
    }

    #[inline]
    pub(crate) fn read_without_context<T: Readable<'a>>(&mut self) -> Option<T> {
        self.read::<T>(ReaderContext::new(&XRef::dummy(), true))
    }

    #[inline]
    pub(crate) fn skip<T: Skippable>(&mut self, is_content_stream: bool) -> Option<&'a [u8]> {
        let old_offset = self.offset;

        T::skip(self, is_content_stream).or_else(|| {
            self.offset = old_offset;
            None
        })?;

        self.data.get(old_offset..self.offset)
    }

    #[inline]
    pub(crate) fn skip_not_in_content_stream<T: Skippable>(&mut self) -> Option<&'a [u8]> {
        self.skip::<T>(false)
    }

    #[inline]
    pub(crate) fn skip_in_content_stream<T: Skippable>(&mut self) -> Option<&'a [u8]> {
        self.skip::<T>(false)
    }

    #[inline]
    pub(crate) fn skip_bytes(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| {})
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
    pub(crate) fn forward_if(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        if f(self.peek_byte()?) {
            self.forward();

            Some(())
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn forward_while_1(&mut self, f: impl Fn(u8) -> bool) -> Option<()> {
        self.eat(&f)?;
        self.forward_while(f);
        Some(())
    }

    #[inline]
    pub(crate) fn forward_tag(&mut self, tag: &[u8]) -> Option<()> {
        self.peek_tag(tag)?;
        self.offset += tag.len();

        Some(())
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
    pub(crate) fn skip_white_spaces(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_white_space_character(b) {
                self.forward();
            } else {
                return;
            }
        }
    }

    #[inline]
    pub(crate) fn read_white_space(&mut self) -> Option<()> {
        if self.peek_byte()?.is_ascii_whitespace() {
            let w = self.read_byte()?;

            if w == b'\r' && self.peek_byte().is_some_and(|b| b == b'\n') {
                self.read_byte()?;
            }

            return Some(());
        }

        None
    }

    #[inline]
    pub(crate) fn skip_eol_characters(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_eol_character(b) {
                self.forward();
            } else {
                return;
            }
        }
    }

    #[inline]
    pub(crate) fn skip_white_spaces_and_comments(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_white_space_character(b) {
                self.skip_white_spaces()
            } else if b == b'%' {
                Comment::skip(self, true);
            } else {
                return;
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct ReaderContext<'a> {
    pub(crate) xref: &'a XRef,
    pub(crate) in_content_stream: bool,
    pub(crate) obj_number: Option<ObjectIdentifier>,
}

impl<'a> ReaderContext<'a> {
    pub(crate) fn new(xref: &'a XRef, in_content_stream: bool) -> Self {
        Self {
            xref,
            in_content_stream,
            obj_number: None,
        }
    }

    pub(crate) fn dummy() -> Self {
        Self::new(XRef::dummy(), false)
    }
}

pub(crate) trait Readable<'a>: Sized {
    fn read(r: &mut Reader<'a>, ctx: ReaderContext<'a>) -> Option<Self>;
    fn from_bytes(b: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        let xref = XRef::dummy();

        Self::read(&mut r, ReaderContext::new(xref, false))
    }
}

pub(crate) trait Skippable {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()>;
}
