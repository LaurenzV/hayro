# hayro-postscript

[![Crates.io](https://img.shields.io/crates/v/hayro-postscript.svg)](https://crates.io/crates/hayro-postscript)
[![Documentation](https://docs.rs/hayro-postscript/badge.svg)](https://docs.rs/hayro-postscript)

<!-- cargo-rdme start -->

A lightweight PostScript scanner.

This crate provides a scanner for tokenizing PostScript programs into typed objects.
It currently implements a small subset of the PostScript language, focused on what
is needed to support CMap parsing in PDF documents.

### Supported object types
- **Integers** — signed integers, radix numbers (e.g. `8#1777`, `16#FFFE`)
- **Reals** — floating-point numbers with optional exponent (e.g. `34.5`, `1.0E-5`)
- **Names** — literal (`/Name`) and executable (`Name`), with `#XX` hex-escape decoding
- **Strings** — literal `(...)`, hex `<...>`, and ASCII85 `<~...~>`, with lazy decoding
- **Arrays** — `[...]` with lazy inner object iteration

### Limitations
This crate only implements a small subset of the PostScript language. In particular,
the following features are **not** supported:
- Dictionaries (`<<` / `>>`)
- Procedures (`{` / `}`)
- The PostScript execution model (operand/dictionary stacks, operators, etc.)

Encountering unsupported syntax returns an error rather than silently skipping it.

### Safety
This crate forbids unsafe code via a crate-level attribute.

<!-- cargo-rdme end -->

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
