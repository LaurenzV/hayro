use crate::object::ObjectIdentifier;
use crate::object::Stream;
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
#[cfg(feature = "streaming")]
use std::collections::{HashMap, hash_map};

/// A container for the bytes of a PDF file.
#[derive(Clone)]
pub enum PdfData {
    /// Buffer in memory containing the PDF file
    #[cfg(feature = "std")]
    Buffer(Arc<dyn AsRef<[u8]> + Send + Sync>),
    /// Buffer in memory containing the PDF file
    #[cfg(not(feature = "std"))]
    Buffer(Arc<dyn AsRef<[u8]>>),
    /// Trait providing data when needed
    #[cfg(feature = "streaming")]
    Streaming(Arc<dyn StreamingSource>),
}

impl Debug for PdfData {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            PdfData::Buffer(as_ref) => {
                write!(f, "PdfData::Buffer({})", as_ref.as_ref().as_ref().len())
            }
            #[cfg(feature = "streaming")]
            PdfData::Streaming(mutex) => write!(f, "PdfData::Streaming({:?})", mutex),
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
impl<T: StreamingSource> From<T> for PdfData {
    fn from(data: T) -> Self {
        Self::Streaming(Arc::new(data))
    }
}

#[cfg(feature = "streaming")]
impl From<Arc<dyn StreamingSource>> for PdfData {
    fn from(data: Arc<dyn StreamingSource>) -> Self {
        Self::Streaming(data)
    }
}

impl PdfData {
    /// create reader from pdf-data
    pub(crate) fn reader(&self) -> Reader<'_, '_> {
        match self {
            PdfData::Buffer(inner) => Reader::from_slice((**inner).as_ref()),
            #[cfg(feature = "streaming")]
            PdfData::Streaming(read_seek) => Reader::from_streaming_source(read_seek.clone()),
        }
    }

    /// create reader from pdf-data with cache
    #[cfg(all(feature = "streaming", reader_opt_ext_cache))]
    pub(crate) fn reader_with_cache<'c>(
        &self,
        cache: Option<&'c mut ReaderCache>,
    ) -> Reader<'_, 'c> {
        match self {
            PdfData::Buffer(inner) => Reader::from_slice((**inner).as_ref()),
            PdfData::Streaming(read_seek) => match cache {
                Some(cache) => Reader::from_streaming_source_with_cache(read_seek.clone(), cache),
                None => Reader::from_streaming_source(read_seek.clone()),
            },
        }
    }

    /// instantiates cache if necessary
    #[cfg(all(feature = "streaming", reader_opt_ext_cache))]
    pub(crate) fn make_cache(&self) -> Option<ReaderCache> {
        match self {
            PdfData::Buffer(_) => None,
            PdfData::Streaming(_) => Some(ReaderCache::default()),
        }
    }
}

/// Source providing data as needed
#[cfg(feature = "streaming")]
pub trait StreamingSource: Debug + Send + Sync + 'static {
    /// Total length of data in bytes
    fn len(&self) -> std::io::Result<usize>;

    /// Read range of bytes
    ///
    /// Returning `Ok(Some(..))` with read data,
    /// `Ok(None)` when provided range is outside of data,
    /// `Err(..)` when data can't be loaded
    fn read(&self, range: core::ops::Range<usize>) -> std::io::Result<Option<Vec<u8>>>;
}

/// Basic streaming file source with optional cache
#[cfg(feature = "streaming")]
pub struct FileSource {
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(not(unix))]
    file: Mutex<std::fs::File>,
    len: usize,
    cache: Option<(usize, Mutex<HashMap<usize, Vec<u8>>>)>,
}

#[cfg(feature = "streaming")]
impl FileSource {
    /// Streaming file source with optional cache
    ///
    /// * `file` - the file to be read
    /// * `chunk_size` - size of cache chunks (typically 1K, 4K, ...), 0 for no cache
    pub fn new(file: std::fs::File, chunk_size: usize) -> Self {
        let len = file.metadata().unwrap().len().try_into().unwrap();
        Self {
            #[cfg(unix)]
            file: file,
            #[cfg(not(unix))]
            file: Mutex::new(file),
            len: len,
            cache: if chunk_size > 0 {
                Some((chunk_size, Mutex::new(HashMap::new())))
            } else {
                None
            },
        }
    }

    fn read_direct(&self, range: core::ops::Range<usize>) -> std::io::Result<Option<Vec<u8>>> {
        if range.end > self.len {
            return Ok(None);
        }
        let len = range.end - range.start;
        if len == 0 {
            return Ok(Some(Vec::new()));
        }
        let offset = range.start.try_into().unwrap();
        let mut buf = vec![0; len];
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::read_exact_at(&self.file, &mut buf, offset)?;
        }
        #[cfg(not(unix))]
        {
            use std::io::{Read, Seek};
            let mut file = self.file.lock().unwrap();
            file.seek(std::io::SeekFrom::Start(offset))?;
            file.read_exact(&mut buf)?;
        }
        Ok(Some(buf))
    }

    fn read_cached(
        &self,
        range: core::ops::Range<usize>,
        chunk_size: usize,
        cache: &Mutex<HashMap<usize, Vec<u8>>>,
    ) -> std::io::Result<Option<Vec<u8>>> {
        if range.end > self.len {
            return Ok(None);
        }
        let len = range.end - range.start;
        if len == 0 {
            return Ok(Some(Vec::new()));
        }
        let mut cache_map = cache.lock().unwrap();
        let mut out = Vec::with_capacity(len);
        let start_chunk = range.start / chunk_size;
        let start_offset = range.start % chunk_size;
        let end_chunk = (range.end - 1) / chunk_size;
        let end_offset = ((range.end - 1) % chunk_size) + 1;
        for chunk in start_chunk..=end_chunk {
            let buffer = cache_map.entry(chunk);
            let cached = match buffer {
                hash_map::Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
                hash_map::Entry::Vacant(vacant_entry) => {
                    let read_start = chunk * chunk_size;
                    let read_end = (read_start + chunk_size).min(self.len);
                    let data = self.read_direct(read_start..read_end)?.ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "reading chunk returned out-of-bounds",
                        )
                    })?;
                    vacant_entry.insert(data)
                }
            };

            let slice = match (chunk == start_chunk, chunk == end_chunk) {
                (true, true) => &cached[start_offset..end_offset],
                (true, false) => &cached[start_offset..],
                (false, true) => &cached[..end_offset],
                (false, false) => cached,
            };
            out.extend_from_slice(slice);
        }
        Ok(Some(out))
    }
}

#[cfg(feature = "streaming")]
impl StreamingSource for FileSource {
    fn len(&self) -> std::io::Result<usize> {
        Ok(self.len)
    }

    fn read(&self, range: core::ops::Range<usize>) -> std::io::Result<Option<Vec<u8>>> {
        if let Some((chunk_size, cache)) = &self.cache {
            self.read_cached(range, *chunk_size, cache)
        } else {
            self.read_direct(range)
        }
    }
}

#[cfg(feature = "streaming")]
impl Debug for FileSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        #[cfg(unix)]
        let file = &self.file;
        #[cfg(not(unix))]
        let file = self.file.lock().unwrap();
        f.debug_struct("FileSource")
            .field("file", &file)
            .field("len", &self.len)
            .field(
                "cache",
                &self
                    .cache
                    .as_ref()
                    .map(|(chunk_size, cache_map)| (chunk_size, cache_map.lock().unwrap().len())),
            )
            .finish()
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
