//! The starting point for reading PDF files.

use crate::PdfData;
use crate::document::page::{Page, Pages};
use crate::object::Object;
use crate::reader::{Reader, ReaderContext};
use crate::xref::{XRef, XRefError, fallback, root_xref};

/// A PDF file.
pub struct Pdf {
    xref: XRef,
    header_version: f32,
}

impl Pdf {
    /// Try to read the given PDF file.
    ///
    /// Returns `None` if it was unable to read it.
    pub fn new(data: PdfData) -> Option<Self> {
        let version = find_version(data.as_ref().as_ref()).unwrap_or(1.0);
        let xref = match root_xref(data.clone()) {
            Ok(x) => x,
            Err(e) => match e {
                XRefError::Unknown => fallback(data)?,
                XRefError::Encrypted => return None,
            },
        };

        Some(Self {
            xref,
            header_version: version,
        })
    }

    /// Return the number of objects present in the PDF file.
    pub fn len(&self) -> usize {
        self.xref.len()
    }

    /// Return an iterator over all objects defined in the PDF file.
    pub fn objects(&self) -> impl IntoIterator<Item = Object> {
        self.xref.objects()
    }

    /// Return the version of the PDF file.
    pub fn version(&self) -> f32 {
        self.xref
            .trailer_data()
            .version
            .unwrap_or(self.header_version)
    }

    /// Return the pages of the PDF file.
    pub fn pages(&self) -> Option<Pages> {
        let ctx = ReaderContext::new(&self.xref, false);
        self.xref
            .get(self.xref.trailer_data().pages_ref)
            .and_then(|p| Pages::new(p, ctx, &self.xref))
    }
}

fn find_version(data: &[u8]) -> Option<f32> {
    let data = &data[..data.len().min(2000)];
    let mut r = Reader::new(data);

    while r.forward_tag(b"%PDF-").is_none() {
        r.read_byte()?;
    }

    r.read_without_context::<f32>()
}

#[cfg(test)]
mod tests {
    use crate::pdf::Pdf;
    use std::sync::Arc;

    #[test]
    fn issue_49() {
        let data = Arc::new([]);
        let pdf = Pdf::new(data);
    }

    #[test]
    fn pdf_version() {
        let data = std::fs::read("../hayro-tests/pdfs/pdfjs/alphatrans.pdf").unwrap();
        let pdf = Pdf::new(Arc::new(data)).unwrap();

        assert_eq!(pdf.version(), 1.7);
    }
}
