# hayro-ccitt

[![Crates.io](https://img.shields.io/crates/v/hayro-ccitt.svg)](https://crates.io/crates/hayro-ccitt)
[![Documentation](https://docs.rs/hayro-ccitt/badge.svg)](https://docs.rs/hayro-ccitt)

<!-- cargo-rdme start -->

A decoder for CCITT fax-encoded images.

This crate implements the CCITT Group 3 and Group 4 fax compression algorithms
as defined in ITU-T Recommendations T.4 and T.6. These encodings are commonly
used for bi-level (black and white) images in PDF documents and fax transmissions.

The main entry point is the [decode] function, which takes encoded data and
decoding settings, and outputs the decoded pixels through a [Decoder] trait.

## Safety
Unsafe code is forbidden via a crate-level attribute.

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

<!-- cargo-rdme end -->
