# hayro

[![Crates.io](https://img.shields.io/crates/v/hayro.svg)](https://crates.io/crates/hayro)
[![Documentation](https://docs.rs/hayro/badge.svg)](https://docs.rs/hayro)

<!-- cargo-rdme start -->

A crate for rendering PDF files.

This crate allows you to render pages of a PDF file into bitmaps. It is supposed to be relatively
lightweight, since we do not have any dependencies on the GPU. All the rendering happens on the CPU.

The ultimate goal of this crate is to be a *feature-complete* and *performant* PDF rasterizer.
While there is still work to be done on both fronts, this crate is currently the most comprehensive and feature-complete
implementation of a PDF rasterizer in pure Rust. This claim is supported by the fact that we currently
include over 1500 PDF files in our regression test suite. The majority of those have been scraped
from the `pdf.js` and `PDFBOX` test suites and therefore represent a very large and diverse sample
of PDF files.

There are still some limitations, including lack of support for isolated and knockout
transparency groups as well as a range of smaller features. But you should be able to render the vast majority
of PDF files without too many issues.

### Safety
This crate forbids unsafe code via a crate-level attribute.

### Examples
For usage examples, see the [example](https://github.com/LaurenzV/hayro/tree/main/hayro/examples) in
the GitHub repository.

### Cargo features
This crate has two default features:
- `embed-fonts`: See the description of [`hayro-interpret`](https://docs.rs/hayro-interpret/latest/hayro_interpret/#cargo-features) for more information.
- `embed-cmaps`: See the description of [`hayro-interpret`](https://docs.rs/hayro-interpret/latest/hayro_interpret/#cargo-features) for more information.

<!-- cargo-rdme end -->

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option. 
