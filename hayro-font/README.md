# hayro-font

[![Crates.io](https://img.shields.io/crates/v/hayro-font.svg)](https://crates.io/crates/hayro-font)
[![Documentation](https://docs.rs/hayro-font/badge.svg)](https://docs.rs/hayro-font)

<!-- cargo-rdme start -->

Parsing the fonts of PDF files.

This crate turns PDF font dictionaries, together with their embedded font programs,
into fonts that can map character codes to glyphs and draw the outlines of those glyphs.

### Safety
This crate forbids unsafe code via a crate-level attribute.

### Cargo features
- `embed-fonts`: Embed a light-weight set of fonts (around 240KB) that can be used as a
  substitute for the 14 PDF standard fonts.

<!-- cargo-rdme end -->

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

The pfb files in `assets` were extracted from Pdfium, see `assets/LICENSE_FOXIT`.
