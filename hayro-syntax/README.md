# hayro-syntax

[![Crates.io](https://img.shields.io/crates/v/hayro-syntax.svg)](https://crates.io/crates/hayro-syntax)
[![Documentation](https://docs.rs/hayro-syntax/badge.svg)](https://docs.rs/hayro-syntax)

<!-- cargo-rdme start -->

A low-level library for reading PDF files.

This crate implements the `Syntax` chapter of the PDF reference, and therefore
serves as a very good basis for building various abstractions on top of it, without having to reimplement
the PDF parsing logic.

This crate does not provide more high-level functionality, such as parsing fonts or color spaces.
Such functionality is out-of-scope for `hayro-syntax`, since this crate is supposed to be
as *light-weight* and *application-agnostic* as possible.

Functionality-wise, this crate is therefore close to feature-complete. The main missing feature
is support for password-protected documents. In addition to that, more low-level APIs might be 
added in the future.

## Example
This short example shows you how to load a PDF file and iterate over the content streams of all
pages.
```rust
use hayro_syntax::Pdf;
use std::path::PathBuf;
use std::sync::Arc;

// First load the data that constitutes the PDF file.
let data = std::fs::read(
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hayro-tests/pdfs/custom/text_with_rise.pdf"),
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

## Safety
There is one usage of `unsafe`, needed to implement caching using a self-referential struct. Other
than that, there is no usage of `unsafe`, especially in _any_ of the parser code.

## Features
The supported features include:
- Parsing xref tables in all its possible formats, including xref streams.
- Best-effort attempt at repairing PDF files with broken xref tables.
- Parsing of all objects types (also in object streams).
- Parsing and decoding PDF streams.
- Iterating over pages as well as their content streams in a typed or untyped fashion.
- The crate is very lightweight, especially in comparison to other PDF crates.

## Limitations
- There are still a few features missing, for example, support for
  password-protected PDFs. In addition to that, many properties (like page annotations) are
  currently not exposed.
- This crate is for read-only processing, you cannot directly use it to manipulate PDF files.
  If you need to do that, there are other crates in the Rust ecosystem that are suitable for this.

<!-- cargo-rdme end -->

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option. 
