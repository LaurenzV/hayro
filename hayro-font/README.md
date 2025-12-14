# hayro-font

This crate is a fork of the [`ttf-parser`](https://github.com/harfbuzz/ttf-parser) library, but with the majority of the functionality completely stripped away. The purpose of this crate is to be a light-weight font parser for CFF and Type1 fonts, as they can be found in PDFs. Only the code for parsing CFF fonts has been retained, while code for parsing Type1 fonts was newly added.

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option. 