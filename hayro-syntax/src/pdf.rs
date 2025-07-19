//! The starting point for reading PDF files.

use crate::PdfData;
use crate::object::Object;
use crate::page::Pages;
use crate::page::cached::CachedPages;
use crate::reader::Reader;
use crate::xref::{XRef, XRefError, fallback, root_xref};
use std::sync::Arc;

/// A PDF file.
pub struct Pdf {
    xref: Arc<XRef>,
    header_version: PdfVersion,
    pages: CachedPages,
    data: PdfData,
}

/// An error that occurred while loading a PDF file.
#[derive(Debug, Copy, Clone)]
pub enum LoadPdfError {
    /// The PDF was encrypted. Encrypted PDF files are currently not supported.
    Encryption,
    /// The PDF was invalid or could not be parsed due to some other unknown reason.
    Invalid,
}

#[allow(clippy::len_without_is_empty)]
impl Pdf {
    /// Try to read the given PDF file.
    ///
    /// Returns `None` if it was unable to read it.
    pub fn new(data: PdfData) -> Result<Self, LoadPdfError> {
        let version = find_version(data.as_ref().as_ref()).unwrap_or(PdfVersion::Pdf10);
        let xref = match root_xref(data.clone()) {
            Ok(x) => x,
            Err(e) => match e {
                XRefError::Unknown => fallback(data.clone()).ok_or(LoadPdfError::Invalid)?,
                XRefError::Encrypted => return Err(LoadPdfError::Encryption),
            },
        };
        let xref = Arc::new(xref);

        let pages = CachedPages::new(xref.clone()).ok_or(LoadPdfError::Invalid)?;

        Ok(Self {
            xref,
            header_version: version,
            pages,
            data,
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
    pub fn version(&self) -> PdfVersion {
        self.xref
            .trailer_data()
            .version
            .unwrap_or(self.header_version)
    }

    /// Return the underlying data of the PDF file.
    pub fn data(&self) -> &PdfData {
        &self.data
    }

    /// Return the pages of the PDF file.
    pub fn pages(&self) -> &Pages {
        self.pages.get()
    }

    /// Return the xref of the PDF file.
    pub fn xref(&self) -> &XRef {
        &self.xref
    }
}

fn find_version(data: &[u8]) -> Option<PdfVersion> {
    let data = &data[..data.len().min(2000)];
    let mut r = Reader::new(data);

    while r.forward_tag(b"%PDF-").is_none() {
        r.read_byte()?;
    }

    PdfVersion::from_bytes(r.tail()?)
}

/// The version of a PDF document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PdfVersion {
    /// PDF 1.0.
    Pdf10,
    /// PDF 1.1.
    Pdf11,
    /// PDF 1.2.
    Pdf12,
    /// PDF 1.3.
    Pdf13,
    /// PDF 1.4.
    Pdf14,
    /// PDF 1.5.
    Pdf15,
    /// PDF 1.6.
    Pdf16,
    /// PDF 1.7.
    Pdf17,
    /// PDF 2.0.
    Pdf20,
}

impl PdfVersion {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<PdfVersion> {
        match bytes.get(..3)? {
            b"1.0" => Some(PdfVersion::Pdf10),
            b"1.1" => Some(PdfVersion::Pdf11),
            b"1.2" => Some(PdfVersion::Pdf12),
            b"1.3" => Some(PdfVersion::Pdf13),
            b"1.4" => Some(PdfVersion::Pdf14),
            b"1.5" => Some(PdfVersion::Pdf15),
            b"1.6" => Some(PdfVersion::Pdf16),
            b"1.7" => Some(PdfVersion::Pdf17),
            b"2.0" => Some(PdfVersion::Pdf20),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pdf::{Pdf, PdfVersion};
    use std::sync::Arc;

    #[test]
    fn issue_49() {
        let data = Arc::new([]);
        let _ = Pdf::new(data);
    }

    #[test]
    fn pdf_version_header() {
        let data = std::fs::read("../hayro-tests/pdfs/pdfjs/alphatrans.pdf").unwrap();
        let pdf = Pdf::new(Arc::new(data)).unwrap();

        assert_eq!(pdf.version(), PdfVersion::Pdf17);
    }

    #[test]
    fn pdf_version_catalog() {
        let data = std::fs::read("../hayro-tests/downloads/pdfbox/2163.pdf").unwrap();
        let pdf = Pdf::new(Arc::new(data)).unwrap();

        assert_eq!(pdf.version(), PdfVersion::Pdf14);
    }
}
