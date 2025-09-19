use crate::PdfData;
use crate::object::ObjectIdentifier;
use crate::object::Stream;
use crate::reader::ReaderContext;
use crate::util::SegmentList;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::Mutex;

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
    map: Mutex<HashMap<ObjectIdentifier, usize>>,
}

impl Debug for Data {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Data {{ ... }}")
    }
}

impl Data {
    /// Create a new `Data` structure.
    pub fn new(data: PdfData) -> Self {
        Self {
            data,
            decoded: SegmentList::new(),
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Get access to the original data of the PDF.
    pub(crate) fn get(&self) -> &[u8] {
        self.data.as_ref().as_ref()
    }

    /// Get access to the data of a decoded object stream.
    pub(crate) fn get_with(&self, id: ObjectIdentifier, ctx: &ReaderContext) -> Option<&[u8]> {
        if let Some(&idx) = self.map.lock().unwrap().get(&id) {
            self.decoded.get(idx)?.as_deref()
        } else {
            // Block scope to keep the lock short-lived.
            let idx = {
                let mut locked = self.map.lock().unwrap();
                let idx = locked.len();
                locked.insert(id, idx);
                idx
            };
            self.decoded
                .get_or_init(idx, || {
                    let stream = ctx.xref.get_with::<Stream>(id, ctx)?;
                    stream.decoded().ok()
                })
                .as_deref()
        }
    }
}
