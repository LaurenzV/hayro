# hayro-postscript

[![Crates.io](https://img.shields.io/crates/v/hayro-postscript.svg)](https://crates.io/crates/hayro-postscript)
[![Documentation](https://docs.rs/hayro-postscript/badge.svg)](https://docs.rs/hayro-postscript)

<!-- cargo-rdme start -->

A lightweight PostScript scanner.

This crate provides a scanner for tokenizing PostScript programs into typed objects.
It currently only implements a very small subset of the PostScript language, 
with the main goal of being enough to parse CMAP files, but the scope _might_
be expanded upon in the future.

The supported types include integers and real numbers, name objects, strings and arrays.
Unsupported is anything else, including dictionaries, procedures, etc. An error
will be returned in case any of these is encountered.

### Safety
This crate forbids unsafe code via a crate-level attribute.

<!-- cargo-rdme end -->

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
