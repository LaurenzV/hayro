//! Reading document metadata.

use crate::object::DateTime;
use alloc::boxed::Box;

#[derive(Clone, Default, Debug, PartialEq, Eq)]
/// The metadata of a PDF document.
pub struct Metadata {
    /// The creation date of the document.
    pub creation_date: Option<DateTime>,
    /// The modification date of the document.
    pub modification_date: Option<DateTime>,
    /// The title of the document.
    pub title: Option<Box<[u8]>>,
    /// The author of the document.
    pub author: Option<Box<[u8]>>,
    /// The subject of the document.
    pub subject: Option<Box<[u8]>>,
    /// The keywords of the document.
    pub keywords: Option<Box<[u8]>>,
    /// The creator of the document.
    pub creator: Option<Box<[u8]>>,
    /// The producer of the document.
    pub producer: Option<Box<[u8]>>,
}
