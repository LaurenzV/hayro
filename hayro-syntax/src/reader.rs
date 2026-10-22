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
    fn skip_with_depth<T: Skippable>(
        &mut self,
        is_content_stream: bool,
        depth: usize,
    ) -> Option<&'a [u8]>;
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
        self.read::<T>(ReaderContext::dummy_ref())
    }

    #[inline]
    fn skip<T: Skippable>(&mut self, is_content_stream: bool) -> Option<&'a [u8]> {
        self.skip_with_depth::<T>(is_content_stream, 0)
    }

    #[inline]
    fn skip_with_depth<T: Skippable>(
        &mut self,
        is_content_stream: bool,
        depth: usize,
    ) -> Option<&'a [u8]> {
        let old_offset = self.offset;

        T::skip_with_depth(self, is_content_stream, depth).or_else(|| {
            self.offset = old_offset;
            None
        })?;

        self.data.get(old_offset..self.offset)
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
        if is_white_space_character(self.peek_byte()?) {
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
        skip_white_spaces_and_comments_in(self, &Unknown);
    }
}

/// Where literal strings and comments end, if that is known without reading them.
///
/// Rebuilding the xref table reads objects from many starting points inside one
/// another, so it looks the ends up instead of reading each string or comment again
/// from every start; all other reading uses [`Unknown`], which compiles to reading them.
pub(crate) trait Ends {
    /// Where the literal string whose `(` is at `start` of `data` ends, after its `)`, or
    /// `Some(None)` if it doesn't end.
    fn string_end(&self, data: &[u8], start: usize) -> Option<Option<usize>>;
    /// Where the comment whose `%` is at `start` of `data` ends, at the first end-of-line
    /// byte after it or at the end of the data.
    fn comment_end(&self, data: &[u8], start: usize) -> Option<usize>;
    /// Notes that a string or comment of `len` bytes was read because its end wasn't known.
    fn read(&self, len: usize);
}

/// No ends are known.
pub(crate) struct Unknown;

impl Ends for Unknown {
    #[inline(always)]
    fn string_end(&self, _: &[u8], _: usize) -> Option<Option<usize>> {
        None
    }

    #[inline(always)]
    fn comment_end(&self, _: &[u8], _: usize) -> Option<usize> {
        None
    }

    #[inline(always)]
    fn read(&self, _: usize) {}
}

#[inline]
pub(crate) fn skip_white_spaces_and_comments_in(r: &mut Reader<'_>, ends: &impl Ends) {
    while let Some(b) = r.peek_byte() {
        if is_white_space_character(b) {
            r.forward();
        } else if b == b'%' {
            Comment::skip_in(r, ends);
        } else {
            return;
        }
    }
}

/// Runs `skip` on `r`, and moves `r` back to where it was if `skip` fails, as
/// [`ReaderExt::skip`] does.
#[inline]
pub(crate) fn skip_or_stay<'a>(
    r: &mut Reader<'a>,
    skip: impl FnOnce(&mut Reader<'a>) -> Option<()>,
) -> Option<()> {
    let old_offset = r.offset;

    skip(r).or_else(|| {
        r.offset = old_offset;
        None
    })
}

#[derive(Clone, Debug)]
struct ReaderContextData<'a> {
    xref: &'a XRef,
    in_content_stream: bool,
    in_object_stream: bool,
    obj_number: Option<ObjectIdentifier>,
    parent_chain: SmallVec<[ObjectIdentifier; 8]>,
}

#[derive(Clone, Debug)]
enum ReaderContextInner<'a> {
    Shared(Arc<ReaderContextData<'a>>),
    Dummy { in_content_stream: bool },
}

/// Context for reading PDF objects.
#[derive(Clone, Debug)]
pub struct ReaderContext<'a>(ReaderContextInner<'a>);

impl<'a> ReaderContext<'a> {
    pub(crate) fn new(xref: &'a XRef, in_content_stream: bool) -> Self {
        Self(ReaderContextInner::Shared(Arc::new(ReaderContextData {
            xref,
            in_content_stream,
            obj_number: None,
            in_object_stream: false,
            parent_chain: smallvec![],
        })))
    }

    pub fn dummy() -> Self {
        Self(ReaderContextInner::Dummy {
            in_content_stream: false,
        })
    }

    pub(crate) fn dummy_ref() -> &'static Self {
        &Self(ReaderContextInner::Dummy {
            in_content_stream: true,
        })
    }

    #[inline]
    pub(crate) fn xref(&self) -> &'a XRef {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.xref,
            ReaderContextInner::Dummy { .. } => XRef::dummy(),
        }
    }

    #[inline]
    pub(crate) fn in_content_stream(&self) -> bool {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.in_content_stream,
            ReaderContextInner::Dummy { in_content_stream } => *in_content_stream,
        }
    }

    #[inline]
    pub(crate) fn in_object_stream(&self) -> bool {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.in_object_stream,
            ReaderContextInner::Dummy { .. } => false,
        }
    }

    #[inline]
    pub(crate) fn obj_number(&self) -> Option<ObjectIdentifier> {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.obj_number,
            ReaderContextInner::Dummy { .. } => None,
        }
    }

    #[inline]
    pub(crate) fn set_obj_number(&mut self, id: ObjectIdentifier) {
        match &mut self.0 {
            ReaderContextInner::Shared(inner) => Arc::make_mut(inner).obj_number = Some(id),
            ReaderContextInner::Dummy { .. } => {}
        }
    }

    #[inline]
    pub(crate) fn set_in_content_stream(&mut self, val: bool) {
        match &mut self.0 {
            ReaderContextInner::Shared(inner) => Arc::make_mut(inner).in_content_stream = val,
            ReaderContextInner::Dummy { in_content_stream } => *in_content_stream = val,
        }
    }

    #[inline]
    pub(crate) fn set_in_object_stream(&mut self, val: bool) {
        match &mut self.0 {
            ReaderContextInner::Shared(inner) => Arc::make_mut(inner).in_object_stream = val,
            ReaderContextInner::Dummy { .. } => {}
        }
    }

    #[inline]
    pub(crate) fn parent_chain_contains(&self, id: &ObjectIdentifier) -> bool {
        match &self.0 {
            ReaderContextInner::Shared(inner) => inner.parent_chain.contains(id),
            ReaderContextInner::Dummy { .. } => false,
        }
    }

    #[inline]
    pub(crate) fn parent_chain_push(&mut self, id: ObjectIdentifier) {
        match &mut self.0 {
            ReaderContextInner::Shared(inner) => Arc::make_mut(inner).parent_chain.push(id),
            ReaderContextInner::Dummy { .. } => {}
        }
    }
}

pub trait Readable<'a>: Sized {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self>;
    fn from_bytes_impl(b: &'a [u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        Self::read(&mut r, &ReaderContext::dummy())
    }
}

pub trait Skippable {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()>;

    fn skip_with_depth(r: &mut Reader<'_>, is_content_stream: bool, _: usize) -> Option<()> {
        Self::skip(r, is_content_stream)
    }
}
