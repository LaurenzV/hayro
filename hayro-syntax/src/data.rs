use crate::object::ObjectIdentifier;
use crate::object::Stream;
use crate::reader::ReaderContext;
use crate::sync::FxHashMap;
use crate::sync::{Arc, Mutex, MutexExt};
use crate::util::SegmentList;
use alloc::borrow::Cow;
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
        // Resolve the slot index with a single locked check-or-insert, and
        // always go through `get_or_init` afterwards. The map insert and
        // the slot initialization are not atomic, so a concurrent reader
        // can observe the map entry while the inserting thread is still
        // decoding the stream; it has to block in `get_or_init` until the
        // slot is initialized. Reading the slot with the non-blocking
        // `SegmentList::get` at that point would silently treat every
        // object in the object stream as null. The single check-or-insert
        // also keeps two racing threads from inserting the same id twice.
        //
        // Block scope to keep the lock short-lived; in particular, it must
        // not be held while decoding.
        let idx = {
            let mut locked = self.map.get();
            match locked.get(&id) {
                Some(&idx) => idx,
                None => {
                    let idx = locked.len();
                    locked.insert(id, idx);
                    idx
                }
            }
        };
        self.decoded
            .get_or_init(idx, || {
                let stream = ctx.xref().get_with::<Stream<'_>>(id, ctx)?;
                stream.decoded().ok().map(Cow::into_owned)
            })
            .as_deref()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use crate::object::ObjectIdentifier;
    use crate::object::dict::Dict;
    use crate::pdf::Pdf;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The number of objects stored inside the object stream.
    const NUM_MEMBERS: usize = 12;
    /// The object number of the first object inside the object stream.
    const FIRST_MEMBER: usize = 5;

    /// Build an uncompressed PDF whose objects `5..(5 + NUM_MEMBERS)` live in
    /// an object stream (object 3), referenced from a cross-reference
    /// stream (object 4). The object stream carries `padding` bytes of
    /// trailing white space so that decoding it takes long enough for
    /// concurrent readers to actually race the decode.
    fn objstm_pdf(padding: usize) -> Vec<u8> {
        // The members are `<< /V 105 >>`, `<< /V 106 >>`, ...
        let mut pairs = String::new();
        let mut payload = String::new();
        for i in 0..NUM_MEMBERS {
            let obj_num = FIRST_MEMBER + i;
            pairs.push_str(&format!("{obj_num} {} ", payload.len()));
            payload.push_str(&format!("<< /V {} >>\n", 100 + obj_num));
        }
        let first = pairs.len();
        let stream_len = first + payload.len() + padding;

        let mut pdf: Vec<u8> = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.5\n");

        let catalog_offset = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let pages_offset = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");

        let objstm_offset = pdf.len();
        pdf.extend_from_slice(
            format!(
                "3 0 obj\n<< /Type /ObjStm /N {NUM_MEMBERS} /First {first} /Length {stream_len} >>\nstream\n"
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(pairs.as_bytes());
        pdf.extend_from_slice(payload.as_bytes());
        pdf.extend(core::iter::repeat_n(b' ', padding));
        pdf.extend_from_slice(b"\nendstream\nendobj\n");

        // The cross-reference stream (object 4).
        let size = FIRST_MEMBER + NUM_MEMBERS;
        let xref_offset = pdf.len();
        let mut entries: Vec<u8> = Vec::new();
        let normal = |entries: &mut Vec<u8>, offset: usize| {
            entries.push(1);
            entries.extend_from_slice(&(offset as u32).to_be_bytes());
            entries.extend_from_slice(&[0, 0]);
        };
        // Object 0 is free.
        entries.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0]);
        normal(&mut entries, catalog_offset);
        normal(&mut entries, pages_offset);
        normal(&mut entries, objstm_offset);
        normal(&mut entries, xref_offset);
        for i in 0..NUM_MEMBERS {
            // Type 2: stored in object stream 3 at index i.
            entries.push(2);
            entries.extend_from_slice(&3_u32.to_be_bytes());
            entries.extend_from_slice(&(i as u16).to_be_bytes());
        }

        pdf.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /XRef /Size {size} /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n",
                entries.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&entries);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        pdf.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF").as_bytes());

        pdf
    }

    /// All threads resolve the members of one shared object stream at the
    /// same time. Whichever thread inserts the map entry decodes the
    /// stream; every other thread must block until the decoded data is
    /// available instead of silently resolving the members as null.
    #[test]
    fn concurrent_object_stream_resolution_is_not_null() {
        const THREADS: usize = 8;
        const ITERATIONS: usize = 32;

        let data = objstm_pdf(2_000_000);
        // Make sure the file itself is well-formed.
        assert_eq!(Pdf::new(data.clone()).unwrap().xref().len(), 16);

        let wrong = AtomicUsize::new(0);

        for _ in 0..ITERATIONS {
            let pdf = Pdf::new(data.clone()).unwrap();
            let barrier = Barrier::new(THREADS);

            std::thread::scope(|s| {
                for _ in 0..THREADS {
                    s.spawn(|| {
                        barrier.wait();

                        for i in 0..NUM_MEMBERS {
                            let obj_num = (FIRST_MEMBER + i) as i32;
                            let id = ObjectIdentifier::new(obj_num, 0);
                            let value = pdf
                                .xref()
                                .get::<Dict<'_>>(id)
                                .and_then(|d| d.get::<i32>(b"V"));

                            if value != Some(100 + obj_num) {
                                wrong.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    });
                }
            });
        }

        assert_eq!(
            wrong.load(Ordering::Relaxed),
            0,
            "some object stream members resolved incorrectly under concurrency"
        );
    }
}
