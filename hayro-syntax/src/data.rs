use crate::object::ObjectIdentifier;
use crate::object::Stream;
#[cfg(feature = "streaming")]
use crate::reader::CustomSource;
use crate::reader::Reader;
#[cfg(all(feature = "streaming", reader_opt_ext_cache))]
use crate::reader::ReaderCache;
use crate::reader::ReaderContext;
use crate::sync::FxHashMap;
use crate::sync::{Arc, Mutex, MutexExt};
use crate::util::SegmentList;
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt::{Debug, Formatter};

/// A container for the bytes of a PDF file.
#[derive(Clone)]
pub enum PdfData {
    /// Buffer in memory
    #[cfg(feature = "std")]
    Buffer(Arc<dyn AsRef<[u8]> + Send + Sync>),
    /// Buffer in memory
    #[cfg(not(feature = "std"))]
    Buffer(Arc<dyn AsRef<[u8]>>),
    /// Custom Reader
    #[cfg(feature = "streaming")]
    Custom(Arc<Mutex<dyn CustomSource + Send + Sync>>),
}

impl Debug for PdfData {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            PdfData::Buffer(as_ref) => write!(f, "PdfData::Buffer({})", as_ref.as_ref().as_ref().len()),
            #[cfg(feature = "streaming")]
            PdfData::Custom(mutex) => write!(f, "PdfData::Custom({:?})", mutex.lock().unwrap()),
        }
    }
}

#[cfg(feature = "std")]
impl<T: AsRef<[u8]> + Send + Sync + 'static> From<Arc<T>> for PdfData {
    fn from(data: Arc<T>) -> Self {
        Self::Buffer(data)
    }
}

#[cfg(not(feature = "std"))]
impl<T: AsRef<[u8]> + 'static> From<Arc<T>> for PdfData {
    fn from(data: Arc<T>) -> Self {
        Self::Buffer(data)
    }
}

impl From<Vec<u8>> for PdfData {
    fn from(data: Vec<u8>) -> Self {
        Self::Buffer(Arc::new(data))
    }
}

#[cfg(feature = "streaming")]
impl<T: CustomSource + Send + Sync + 'static> From<T> for PdfData {
    fn from(data: T) -> Self {
        Self::Custom(Arc::new(Mutex::new(data)))
    }
}

impl PdfData {
    /// create reader from pdf-data
    pub fn reader(&self) -> Reader<'_, '_> {
        match self {
            PdfData::Buffer(inner) => Reader::from_slice((**inner).as_ref()),
            #[cfg(feature = "streaming")]
            PdfData::Custom(read_seek) => Reader::from_custom_source(read_seek.clone()),
        }
    }

    /// create reader from pdf-data with cache
    #[cfg(all(feature = "streaming", reader_opt_ext_cache))]
    pub fn reader_with_cache<'c>(&self, cache: Option<&'c mut ReaderCache>) -> Reader<'_, 'c> {
        match self {
            PdfData::Buffer(inner) => Reader::from_slice((**inner).as_ref()),
            PdfData::Custom(read_seek) => match cache {
                Some(cache) => Reader::from_custom_source_with_cache(read_seek.clone(), cache),
                None => Reader::from_custom_source(read_seek.clone()),
            },
        }
    }

    /// instantiates cache if necessary
    #[cfg(all(feature = "streaming", reader_opt_ext_cache))]
    pub fn make_cache(&self) -> Option<ReaderCache> {
        match self {
            PdfData::Buffer(_) => None,
            PdfData::Custom(_) => Some(ReaderCache::default()),
        }
    }
}

/// A structure for storing the data of the PDF.
// To explain further: This crate uses a zero-parse approach, meaning that objects like
// dictionaries or arrays always store the underlying data and parse objects lazily as needed,
// instead of allocating the data and storing it in an owned way. However, the problem is that
// not all data is readily available in the original data of the PDF: Objects can also be
// stored in an object streams, in which case we first need to decode the stream before we can
// access the data.
//
// The purpose of `Data` is to allow us to access the original data as well as maybe decoded data
// by faking the same lifetime, so that we don't run into lifetime issues when dealing with
// PDF objects that actually stem from different data sources.
pub(crate) struct Data {
    data: PdfData,
    // 32 segments are more than enough as we can't have more objects than this.
    decoded: SegmentList<Option<Vec<u8>>, 32>,
    map: Mutex<FxHashMap<ObjectIdentifier, usize>>,
}

impl Debug for Data {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Data {{ ... }}")
    }
}

impl Data {
    /// Create a new `Data` structure.
    pub(crate) fn new(data: PdfData) -> Self {
        Self {
            data,
            decoded: SegmentList::new(),
            map: Mutex::new(FxHashMap::default()),
        }
    }

    /// Get access to the original data of the PDF.
    pub(crate) fn get(&self) -> &PdfData {
        &self.data
    }

    /// Get access to the data of a decoded object stream.
    pub(crate) fn get_with(&self, id: ObjectIdentifier, ctx: &ReaderContext<'_>) -> Option<&[u8]> {
        if let Some(&idx) = self.map.get().get(&id) {
            self.decoded.get(idx)?.as_deref()
        } else {
            // Block scope to keep the lock short-lived.
            let idx = {
                let mut locked = self.map.get();
                let idx = locked.len();
                locked.insert(id, idx);
                idx
            };
            self.decoded
                .get_or_init(idx, || {
                    let stream = ctx.xref().get_with::<Stream<'_>>(id, ctx)?;
                    stream.decoded().ok().map(Cow::into_owned)
                })
                .as_deref()
        }
    }
}
