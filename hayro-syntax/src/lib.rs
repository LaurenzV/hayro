/*!
A low-level library for reading PDF files.

This crate implements most of the points in the `Syntax` chapter of the PDF reference, and therefore
serves as a very good basis for building various abstractions on top of it, without having to reimplement
the PDF parsing logic.

This crate does not provide more high-level functionality, such as parsing fonts or color spaces.
Such functionality is out-of-scope for `hayro-syntax`, since this crate is supposed to be
as light-weight and application-agnostic as possible. Functionality-wise, this crate is therefore
pretty much feature-complete, though more low-level APIs might be added in the future.

# Example
This short example shows how you can iterate over the operations of the content stream of all pages
in a PDF file.
```rust
use std::path::PathBuf;
use std::sync::Arc;
use hayro_syntax::pdf::Pdf;

let data = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro-tests/pdfs/text_with_rise.pdf")).unwrap();
let pdf = Pdf::new(Arc::new(data)).unwrap();
let pages = pdf.pages().unwrap();

for page in pages.get() {
    for op in page.typed_operations() {
        println!("{:?}", op);
    }
}
```

# Features
The supported features include:
- Parsing the xref table in all of its possible formats, including xref streams.
- Parsing of all objects (also in object streams).
- Parsing and evaluating of PDF functions.
- Parsing and decoding PDF streams.
- Iterating over pages in a PDF as well as their content streams in a typed fashion.
- The crate is very lightweight in comparison to other PDF crates, at least if you don't enable
  the jpeg2000 feature.

# Limitations
I would like to highlight the following limitations:

- There are still features missing, for example, support for encrypted PDFs. In addition to that,
  many properties (like page annotations) are currently not exposed.
- This crate is for read-only processing, you cannot directly use it to manipulate PDF files.
  If you need to do that, there are other crates in the Rust ecosystem that are suitable for this.
- I do want to note that the main reason this crate exists is to serve as a foundation for
  `hayro-render`. Therefore, I am not planning on adding many other features that aren't needed
  to rasterize PDFs. But I am open to feedback, and if the crate covers everything
  you need, you are more than free to use it directly!

# Cargo features
This crate has one feature, `jpeg2000`. PDF allows for the insertion of JPEG2000 images. However,
unfortunately, JPEG2000 is a very complicated format. There exists a Rust crate that allows decoding
such images (which is also used by `hayro-syntax`), but it is a very heavy dependency, has a lot of
unsafe code (due to having been ported with `c2rust`), and also has a dependency on libc, meaning that you
might be restricted in the targets you can build to. Because of this, I recommend not enabling this
feature, unless you absolutely need to be able to support such images.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::sync::Arc;

pub mod bit_reader;
pub mod content;
pub(crate) mod data;
pub mod document;
pub mod filter;
pub mod function;
pub mod object;
pub mod pdf;
pub mod reader;
pub mod trivia;
pub(crate) mod util;
pub mod xref;

const NUM_SLOTS: usize = 10000;

/// A container for the bytes of a PDF file.
pub type PdfData = Arc<dyn AsRef<[u8]> + Send + Sync>;
