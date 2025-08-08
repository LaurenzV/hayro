/*!
A low-level library for reading PDF files.

This crate implements the `Syntax` chapter of the PDF reference, and therefore
serves as a very good basis for building various abstractions on top of it, without having to reimplement
the PDF parsing logic.

This crate does not provide more high-level functionality, such as parsing fonts or color spaces.
Such functionality is out-of-scope for `hayro-syntax`, since this crate is supposed to be
as *light-weight* and *application-agnostic* as possible.

Functionality-wise, this crate is therefore close to feature-complete. The main missing feature
is support for encrypted and password-protected documents, as well as improved support for JPEG2000
documents. In addition to that, more low-level APIs might be added in the future.

# Example
This short example shows you how to load a PDF file and iterate over the content streams of all
pages.
```rust
use hayro_syntax::Pdf;
use std::path::PathBuf;
use std::sync::Arc;

// First load the data that constitutes the PDF file.
let data = std::fs::read(
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro/pdfs/text_with_rise.pdf"),
)
.unwrap();

// Then create a new PDF file from it.
//
// Here we are just unwrapping in case reading the file failed, but you
// might instead want to apply proper error handling.
let pdf = Pdf::new(Arc::new(data)).unwrap();

// First access all pages, and then iterate over the operators of each page's
// content stream and print them.
let pages = pdf.pages();
for page in pages.iter() {
    for op in page.typed_operations() {
        println!("{op:?}");
    }
}
```

# Safety
There is one usage of `unsafe`, needed to implement caching using a self-referential struct. Other
than that, there is no usage of `unsafe`, especially in _any_ of the parser code.

# Features
The supported features include:
- Parsing xref tables in all its possible formats, including xref streams.
- Best-effort attempt at repairing PDF files with broken xref tables.
- Parsing of all objects types (also in object streams).
- Parsing and evaluating PDF functions.
- Parsing and decoding PDF streams.
- Iterating over pages as well as their content streams in a typed or untyped fashion.
- The crate is very lightweight, especially in comparison to other PDF crates, assuming you don't
  enable the `jpeg2000` feature (see further below for more information).

# Limitations
- There are still a few features missing, for example, support for encrypted and
  password-protected PDFs. In addition to that, many properties (like page annotations) are
  currently not exposed.
- This crate is for read-only processing, you cannot directly use it to manipulate PDF files.
  If you need to do that, there are other crates in the Rust ecosystem that are suitable for this.

# Cargo features
This crate has one feature, `jpeg2000`. PDF allows for the insertion of JPEG2000 images. However,
unfortunately, JPEG2000 is a very complicated format. There exists a Rust
[jpeg2k](https://github.com/Neopallium/jpeg2k) crate that allows decoding such images. However, it is a
relatively heavy dependency, has a lot of unsafe code (due to having been ported with
[c2rust](https://c2rust.com/)), and also has a dependency on libc, meaning that you might be
restricted in the targets you can build to. Because of this, I recommend not enabling this feature
unless you absolutely need to be able to support such images.
*/

#![deny(missing_docs)]

use std::sync::Arc;

pub(crate) mod data;
pub(crate) mod filter;
pub(crate) mod pdf;
pub(crate) mod reader;
pub(crate) mod trivia;
pub(crate) mod util;

pub mod bit_reader;
pub mod content;
pub mod function;
pub mod object;
pub mod page;
pub mod xref;

pub use pdf::*;

/// A container for the bytes of a PDF file.
pub type PdfData = Arc<dyn AsRef<[u8]> + Send + Sync>;
