use crate::object::ObjectIdentifier;
use crate::object::Stream;
use crate::reader::ReaderContext;
use crate::sync::FxHashMap;
use crate::sync::{Arc, Mutex, MutexExt};
use crate::util::SegmentList;
use crate::xref::ObjectStream;
use alloc::vec::Vec;
use core::fmt::{Debug, Formatter};

/// A container for the bytes of a PDF file.
#[derive(Clone)]
pub struct PdfData {
    #[cfg(feature = "std")]
    inner: Arc<dyn AsRef<[u8]> + Send + Sync>,
    #[cfg(not(feature = "std"))]
    inner: Arc<dyn AsRef<[u8]>>,
}

impl Debug for PdfData {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "PdfData {{ ... }}")
    }
}

impl AsRef<[u8]> for PdfData {
    fn as_ref(&self) -> &[u8] {
        (*self.inner).as_ref()
    }
}

#[cfg(feature = "std")]
impl<T: AsRef<[u8]> + Send + Sync + 'static> From<Arc<T>> for PdfData {
    fn from(data: Arc<T>) -> Self {
        Self { inner: data }
    }
}

#[cfg(not(feature = "std"))]
impl<T: AsRef<[u8]> + 'static> From<Arc<T>> for PdfData {
    fn from(data: Arc<T>) -> Self {
        Self { inner: data }
    }
}

impl From<Vec<u8>> for PdfData {
    fn from(data: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(data),
        }
    }
}

// Gives decoded object streams the same lifetime as the original data.
pub(crate) struct Data {
    data: PdfData,
    // 32 segments are more than enough as we can't have more objects than this.
    object_streams: SegmentList<ObjectStream, 32>,
    map: Mutex<FxHashMap<ObjectIdentifier, usize>>,
}

impl Debug for Data {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Data {{ ... }}")
    }
}

impl Data {
    pub(crate) fn new(data: PdfData) -> Self {
        Self {
            data,
            object_streams: SegmentList::new(),
            map: Mutex::new(FxHashMap::default()),
        }
    }

    pub(crate) fn get(&self) -> &PdfData {
        &self.data
    }

    pub(crate) fn object_stream(
        &self,
        id: ObjectIdentifier,
        ctx: &ReaderContext<'_>,
    ) -> Option<&ObjectStream> {
        if let Some(&idx) = self.map.get().get(&id) {
            return self.object_streams.get(idx);
        }

        // Decoding may resolve other objects, so do it without holding the cache lock.
        let stream = ctx.xref().get_with::<Stream<'_>>(id, ctx)?;
        let object_stream = ObjectStream::new(&stream)?;

        let mut locked = self.map.get();
        if let Some(&idx) = locked.get(&id) {
            return self.object_streams.get(idx);
        }

        let idx = locked.len();
        let object_stream = self.object_streams.get_or_init(idx, || object_stream);
        locked.insert(id, idx);

        Some(object_stream)
    }
}
