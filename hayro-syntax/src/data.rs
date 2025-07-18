use crate::object::ObjectIdentifier;
use crate::object::Stream;
use crate::xref::XRef;
use crate::{NUM_SLOTS, PdfData};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicUsize;

/// A structure for storing the data of the PDF.
// To explain further: This crate uses a zero-parse approach, meaning that objects like
// dictionaries or arrays always store the underlying data and parse objects lazily as needed,
// instead of allocating the data and storing it in an owned way. However, the problem is that
// not all data is readily available: Objects can also be stored in an object stream, in which
// case we first need to decode the stream before we can access the data.
//
// The purpose of `Data` is to allow us to access original data as well as maybe decoded data
// by faking the same lifetime, so that we don't run into lifetime issues when dealing with
// PDF objects that actually stem from different data sources.
pub(crate) struct Data {
    data: PdfData,
    slots: Vec<OnceLock<Option<Vec<u8>>>>,
    map: Mutex<HashMap<ObjectIdentifier, usize>>,
    counter: AtomicUsize,
}

impl Debug for Data {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Data {{ ... }}")
    }
}

impl Data {
    /// Create a new `Data` structure.
    pub fn new(data: PdfData) -> Self {
        let map = Mutex::new(HashMap::new());
        let slots = vec![OnceLock::new(); NUM_SLOTS];
        let counter = AtomicUsize::new(0);

        Self {
            data,
            slots,
            map,
            counter,
        }
    }

    /// Get access to the original data of the PDF.
    pub(crate) fn get(&self) -> &[u8] {
        self.data.as_ref().as_ref()
    }

    /// Get access to the data of a decoded object stream.
    pub(crate) fn get_with(&self, id: ObjectIdentifier, xref: &XRef) -> Option<&[u8]> {
        if let Some(idx) = self.map.lock().unwrap().get(&id) {
            self.slots[*idx].get()?.as_deref()
        } else {
            let mut idx = self.counter.load(std::sync::atomic::Ordering::SeqCst);

            if idx >= NUM_SLOTS {
                None
            } else {
                self.map.lock().unwrap().insert(id, idx);

                let stream = xref.get::<Stream>(id)?;
                self.slots[idx].set(stream.decoded().ok()).unwrap();

                let val = self.slots[idx].get().unwrap().as_deref();
                idx += 1;

                self.counter.store(idx, std::sync::atomic::Ordering::SeqCst);

                val
            }
        }
    }
}
