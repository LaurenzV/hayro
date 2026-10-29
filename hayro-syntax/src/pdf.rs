//! The starting point for reading PDF files.

use crate::PdfData;
use crate::object::Object;
use crate::page::Pages;
use crate::page::cached::CachedPages;
use crate::reader::Reader;
use crate::sync::Arc;
use crate::xref::{ObjectSource, XRef, XRefError, fallback, root_xref};
use alloc::vec::Vec;

pub use crate::crypto::DecryptionError;
use crate::metadata::Metadata;

/// A PDF file.
pub struct Pdf {
    xref: Arc<XRef>,
    header_version: PdfVersion,
    pages: CachedPages,
    data: PdfData,
}

/// An error that occurred while loading a PDF file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LoadPdfError {
    /// An error occurred while processing an encrypted document.
    Decryption(DecryptionError),
    /// The PDF was invalid or could not be parsed due to some other unknown reason.
    Invalid,
}

#[allow(clippy::len_without_is_empty)]
impl Pdf {
    /// Try to read the given PDF file.
    ///
    /// Returns `Err` if it was unable to read it.
    pub fn new(data: impl Into<PdfData>) -> Result<Self, LoadPdfError> {
        Self::new_with_password(data, "")
    }

    /// Try to read the given PDF file with a password.
    ///
    /// Returns `Err` if it was unable to read it or if the password is incorrect.
    pub fn new_with_password(
        data: impl Into<PdfData>,
        password: &str,
    ) -> Result<Self, LoadPdfError> {
        let data = data.into();
        let password = password.as_bytes();
        let version = find_version(data.as_ref()).unwrap_or(PdfVersion::Pdf10);
        let xref = match root_xref(data.clone(), password) {
            Ok(x) => x,
            Err(e) => match e {
                XRefError::Unknown => {
                    fallback(data.clone(), password).ok_or(LoadPdfError::Invalid)?
                }
                XRefError::Encryption(e) => return Err(LoadPdfError::Decryption(e)),
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

    /// Read a PDF from a caller-supplied object source.
    ///
    /// Returns `Err` if the trailer or the page tree cannot be read.
    pub fn from_source(source: Arc<dyn ObjectSource>) -> Result<Self, LoadPdfError> {
        let version = source.header_version().unwrap_or(PdfVersion::Pdf10);
        let xref = Arc::new(XRef::from_source(source).ok_or(LoadPdfError::Invalid)?);
        let pages = CachedPages::new(xref.clone()).ok_or(LoadPdfError::Invalid)?;

        Ok(Self {
            xref,
            header_version: version,
            pages,
            data: PdfData::from(Vec::new()),
        })
    }

    /// Return the number of objects present in the PDF file.
    pub fn len(&self) -> usize {
        self.xref.len()
    }

    /// Return an iterator over all objects defined in the PDF file.
    pub fn objects(&self) -> impl IntoIterator<Item = Object<'_>> {
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
    pub fn pages(&self) -> &Pages<'_> {
        self.pages.get()
    }

    /// Return the xref of the PDF file.
    pub fn xref(&self) -> &XRef {
        &self.xref
    }

    /// Return the metadata in the document information dictionary of the document.
    pub fn metadata(&self) -> &Metadata {
        self.xref.metadata()
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
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        match bytes.get(..3)? {
            b"1.0" => Some(Self::Pdf10),
            b"1.1" => Some(Self::Pdf11),
            b"1.2" => Some(Self::Pdf12),
            b"1.3" => Some(Self::Pdf13),
            b"1.4" => Some(Self::Pdf14),
            b"1.5" => Some(Self::Pdf15),
            b"1.6" => Some(Self::Pdf16),
            b"1.7" => Some(Self::Pdf17),
            b"2.0" => Some(Self::Pdf20),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pdf::{Pdf, PdfVersion};

    use crate::object::{ObjectIdentifier, Stream};
    use crate::sync::{Arc, HashMap};
    use crate::xref::ObjectSource;

    struct MapSource {
        objects: HashMap<ObjectIdentifier, Vec<u8>>,
        trailer: Vec<u8>,
    }

    impl MapSource {
        fn new(objects: &[(i32, &[u8])], trailer: &[u8]) -> Arc<Self> {
            Arc::new(Self {
                objects: objects
                    .iter()
                    .map(|(n, b)| (ObjectIdentifier::new(*n, 0), b.to_vec()))
                    .collect(),
                trailer: trailer.to_vec(),
            })
        }
    }

    impl ObjectSource for MapSource {
        fn object(&self, id: ObjectIdentifier) -> Option<&[u8]> {
            self.objects.get(&id).map(|b| b.as_slice())
        }

        fn trailer(&self) -> &[u8] {
            &self.trailer
        }
    }

    const CATALOG: (i32, &[u8]) = (1, b"<< /Type /Catalog /Pages 2 0 R >>");
    const PAGES: (i32, &[u8]) = (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    const TRAILER: &[u8] = b"<< /Root 1 0 R /Info 5 0 R /Size 6 >>";

    #[test]
    fn source_resolves_pages_and_streams() {
        let source = MapSource::new(
            &[
                CATALOG,
                PAGES,
                (
                    3,
                    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>",
                ),
                (4, b"<< /Length 4 0 R >>\nstream\n0 0 m\nendstream"),
                (5, b"<< /Title (From a source) >>"),
            ],
            TRAILER,
        );
        let pdf = Pdf::from_source(source).unwrap();

        assert_eq!(pdf.pages().len(), 1);
        let page = &pdf.pages()[0];
        assert_eq!(page.base_dimensions(), (200.0, 100.0));
        // A cyclic /Length falls back to scanning for endstream.
        assert_eq!(page.page_stream(), Some(&b"0 0 m"[..]));
        assert_eq!(pdf.metadata().title.as_deref(), Some(&b"From a source"[..]));
        assert!(
            pdf.xref()
                .get::<Stream<'_>>(ObjectIdentifier::new(4, 0))
                .is_some()
        );
    }

    #[test]
    fn source_absent_object_is_null() {
        let source = MapSource::new(
            &[
                CATALOG,
                PAGES,
                (3, b"<< /Type /Page /Parent 2 0 R /Contents 9 0 R >>"),
            ],
            TRAILER,
        );
        let pdf = Pdf::from_source(source).unwrap();

        assert_eq!(pdf.pages().len(), 1);
        assert!(pdf.metadata().title.is_none());
    }

    #[test]
    fn source_without_root_is_invalid() {
        let source = MapSource::new(&[CATALOG, PAGES], b"<< /Size 3 >>");

        assert!(Pdf::from_source(source).is_err());
    }

    #[test]
    fn issue_49() {
        let _ = Pdf::new(Vec::new());
    }

    #[test]
    fn pdf_version_header() {
        let data = std::fs::read("../tests/pdfs/other/version_header.pdf").unwrap();
        let pdf = Pdf::new(data).unwrap();

        assert_eq!(pdf.version(), PdfVersion::Pdf17);
    }

    #[test]
    fn pdf_version_catalog() {
        let data = std::fs::read("../tests/pdfs/other/version_catalog.pdf").unwrap();
        let pdf = Pdf::new(data).unwrap();

        assert_eq!(pdf.version(), PdfVersion::Pdf17);
    }
}
