//! The starting point for reading PDF files.

use crate::PdfData;
use crate::document::page::{Page, Pages};
use crate::object::Object;
use crate::reader::ReaderContext;
use crate::xref::{XRef, XRefError, fallback, root_xref};

/// A PDF file.
pub struct Pdf {
    xref: XRef,
}

impl Pdf {
    /// Try to read the given PDF file.
    ///
    /// Returns `None` if it was unable to read it.
    pub fn new(data: PdfData) -> Option<Self> {
        let xref = match root_xref(data.clone()) {
            Ok(x) => x,
            Err(e) => match e {
                XRefError::Unknown => fallback(data)?,
                XRefError::Encrypted => return None,
            },
        };

        Some(Self { xref })
    }

    /// Return the number of objects present in the PDF file.
    pub fn len(&self) -> usize {
        self.xref.len()
    }

    /// Return an iterator over all objects defined in the PDF file.
    pub fn objects(&self) -> impl IntoIterator<Item = Object> {
        self.xref.objects()
    }

    /// Return the pages of the PDF file.
    pub fn pages(&self) -> Option<Vec<Page>> {
        let ctx = ReaderContext::new(&self.xref, false);
        self.xref
            .get(self.xref.trailer_data().pages_ref)
            .and_then(|p| Pages::new(p, ctx).map(|p| p.pages))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use crate::pdf::Pdf;

    #[test]
    fn issue_49() {
        let data = Arc::new([]);
        let pdf = Pdf::new(data);
    }
}