# hayro-svg

[![Crates.io](https://img.shields.io/crates/v/hayro-svg.svg)](https://crates.io/crates/hayro-svg)
[![Documentation](https://docs.rs/hayro-svg/badge.svg)](https://docs.rs/hayro-svg)

<!-- cargo-rdme start -->

A crate for converting PDF pages to SVG files.

This is the pendant to [`hayro`](https://crates.io/crates/hayro), but allows you to export to
SVG instead of bitmap images. See the description of that crate for more information on the
supported features and limitations.

### Safety
This crate forbids unsafe code via a crate-level attribute.

### Cargo features
This crate has one optional feature:
- `embed-fonts`: See the description of [`hayro-interpret`](https://docs.rs/hayro-interpret/latest/hayro_interpret/#cargo-features) for more information.

<!-- cargo-rdme end -->

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
