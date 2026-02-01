//! Reading bytes and PDF objects from data.

use crate::object::ObjectIdentifier;
use crate::sync::Arc;
use crate::trivia::{Comment, is_eol_character, is_white_space_character};
use crate::xref::XRef;
use smallvec::{SmallVec, smallvec};

pub use crate::byte_reader::Reader;

/// Extension trait for the `Reader` struct.
pub trait ReaderExt<'a> {
    fn read<T: Readable<'a>>(&mut self, ctx: &ReaderContext<'a>) -> Option<T>;
    fn read_with_context<T: Readable<'a>>(&mut self, ctx: &ReaderContext<'a>) -> Option<T>;
    fn read_without_context<T: Readable<'a>>(&mut self) -> Option<T>;
    fn skip<T: Skippable>(&mut self, is_content_stream: bool) -> Option<&'a [u8]>;
    fn skip_not_in_content_stream<T: Skippable>(&mut self) -> Option<&'a [u8]>;
    fn skip_in_content_stream<T: Skippable>(&mut self) -> Option<&'a [u8]>;
    fn skip_white_spaces(&mut self);
    fn read_white_space(&mut self) -> Option<()>;
    fn skip_eol_characters(&mut self);
    fn skip_white_spaces_and_comments(&mut self);
}

impl<'a> ReaderExt<'a> for Reader<'a> {
    // Note: If `PLAIN` is true, it means that the data we are about to read _might_ contain
    // an object reference instead of an actual object. if `PLAIN` is false, then an object
    // reference cannot occur. The main reason we make this distinction is that when parsing
    // a number, we cannot unambiguously distinguish whether it's a real number or the
    // start of an object reference. In content streams, object references cannot appear,
    // so in order to speed this up we set `PLAIN` to false, meaning that as soon as we
    // encounter a number we know it's a number, and don't need to do a look-ahead to ensure
    // that it's not an object reference.
    #[inline]
    fn read<T: Readable<'a>>(&mut self, ctx: &ReaderContext<'a>) -> Option<T> {
        let old_offset = self.offset;

        T::read(self, ctx).or_else(|| {
            self.offset = old_offset;

            None
        })
    }

    #[inline]
    fn read_with_context<T: Readable<'a>>(&mut self, ctx: &ReaderContext<'a>) -> Option<T> {
        self.read::<T>(ctx)
    }

    #[inline]
    fn read_without_context<T: Readable<'a>>(&mut self) -> Option<T> {
        self.read::<T>(&ReaderContext::new(XRef::dummy(), true))
    }

    #[inline]
    fn skip<T: Skippable>(&mut self, is_content_stream: bool) -> Option<&'a [u8]> {
        let old_offset = self.offset;

        T::skip(self, is_content_stream).or_else(|| {
            self.offset = old_offset;
            None
        })?;

        self.data.get(old_offset..self.offset)
    }

    #[inline]
    fn skip_not_in_content_stream<T: Skippable>(&mut self) -> Option<&'a [u8]> {
        self.skip::<T>(false)
    }

    #[inline]
    fn skip_in_content_stream<T: Skippable>(&mut self) -> Option<&'a [u8]> {
        self.skip::<T>(false)
    }

    #[inline]
    fn skip_white_spaces(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_white_space_character(b) {
                self.forward();
            } else {
                return;
            }
        }
    }

    #[inline]
    fn read_white_space(&mut self) -> Option<()> {
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
    fn skip_eol_characters(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_eol_character(b) {
                self.forward();
            } else {
                return;
            }
        }
    }

    #[inline]
    fn skip_white_spaces_and_comments(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_white_space_character(b) {
                self.skip_white_spaces();
            } else if b == b'%' {
                Comment::skip(self, true);
            } else {
                return;
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ReaderContextInner<'a> {
    xref: &'a XRef,
    in_content_stream: bool,
    in_object_stream: bool,
    obj_number: Option<ObjectIdentifier>,
    parent_chain: SmallVec<[ObjectIdentifier; 8]>,
}

/// Context for reading PDF objects.
#[derive(Clone, Debug)]
pub struct ReaderContext<'a>(Arc<ReaderContextInner<'a>>);

impl<'a> ReaderContext<'a> {
    pub(crate) fn new(xref: &'a XRef, in_content_stream: bool) -> Self {
        Self(Arc::new(ReaderContextInner {
            xref,
            in_content_stream,
            obj_number: None,
            in_object_stream: false,
            parent_chain: smallvec![],
        }))
    }

    pub fn dummy() -> Self {
        Self::new(XRef::dummy(), false)
    }

    #[inline]
    pub(crate) fn xref(&self) -> &'a XRef {
        self.0.xref
    }

    #[inline]
    pub(crate) fn in_content_stream(&self) -> bool {
        self.0.in_content_stream
    }

    #[inline]
    pub(crate) fn in_object_stream(&self) -> bool {
        self.0.in_object_stream
    }

    #[inline]
    pub(crate) fn obj_number(&self) -> Option<ObjectIdentifier> {
        self.0.obj_number
    }

    #[inline]
    pub(crate) fn set_obj_number(&mut self, id: ObjectIdentifier) {
        Arc::make_mut(&mut self.0).obj_number = Some(id);
    }

    #[inline]
    pub(crate) fn set_in_content_stream(&mut self, val: bool) {
        Arc::make_mut(&mut self.0).in_content_stream = val;
    }

    #[inline]
    pub(crate) fn set_in_object_stream(&mut self, val: bool) {
        Arc::make_mut(&mut self.0).in_object_stream = val;
    }

    #[inline]
    pub(crate) fn parent_chain_contains(&self, id: &ObjectIdentifier) -> bool {
        self.0.parent_chain.contains(id)
    }

    #[inline]
    pub(crate) fn parent_chain_push(&mut self, id: ObjectIdentifier) {
        Arc::make_mut(&mut self.0).parent_chain.push(id);
    }
}

pub trait Readable<'a>: Sized {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self>;
    fn from_bytes_impl(b: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        let xref = XRef::dummy();

        Self::read(&mut r, &ReaderContext::new(xref, false))
    }
}

pub trait Skippable {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()>;
}
