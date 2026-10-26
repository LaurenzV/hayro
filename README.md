# hayro

[![Crates.io](https://img.shields.io/crates/v/hayro.svg)](https://crates.io/crates/hayro)
[![Documentation](https://docs.rs/hayro/badge.svg)](https://docs.rs/hayro)

A set of crates for reading, interpreting and rendering PDF files.

## Motivation

There are many Rust crates that can parse and process PDF files.

`hayro` was created with the following design decisions in mind, *some* of which distinguish it from
other alternatives:
- **Read-only processing**: This is probably the most important aspect. Libraries like `lopdf`
  convert the PDF into an internal representation, such that it can also be edited and manipulated.
  **`hayro-syntax` provides read-only access**. The advantage of doing this is that it makes parsing
  much faster and more memory-efficient, as we don't need to allocate as much. Of course, the
  downside is that, unlike other crates, you can't manipulate the document. If you need that
  ability, you _still_ can build that logic yourself on top of `hayro-syntax` (or use other crates
  complementarily), but it isn't provided out of the box. For now, providing support for document
  manipulation is out-of-scope, though I might change my mind in the future if there is demand and I
  find time to implement it.
- **Modularity**: An important aspect was to split functionality into crates and allow compiling out
  functionality that is not needed. For example, you can use the JBIG2 and JPEG2000 image decoders
  individually. Additionally, a conscious design decision was to separate the actual parsing of PDF
  files from the conversion into Rust-native data structures, which many other crates combine. If
  you only depend on `hayro-syntax`, you get the bare minimum that is needed to successfully parse
  PDF files, but it doesn't attempt to provide support for parsing fonts or evaluating PDF
  functions. As a result, `hayro-syntax` is a very lightweight dependency.
- **Correctness**: While there are other Rust libraries in development that can render documents, to
  the best of my knowledge `hayro` remains the library with the highest correctness, which is
  underpinned by a visual regression test suite consisting of 1,600+ PDFs, many of which are tricky
  PDFs that have been scraped from the test suite of other PDF engines, like pdf.js or PDFBox.

## Development state
In its current state, you can use `hayro` and basically expect almost any PDF that renders in other
viewers like Chrome (PDFium) or Firefox (pdf.js) to render correctly. Any feature that is expected
to work in most PDF viewers should also work in hayro. This excludes some more exotic features like
knockout groups, CMYK blending or 3D content, which are basically only supported in Acrobat.

The three main remaining aspects that still need work are:
- **Robustness**: While the `hayro` crates already have extensive checks to avoid panics/timeouts on
  untrusted inputs, there are still many ways to trigger timeouts and OOM errors.
- **Performance**: `hayro` can already render many PDFs faster than other PDF viewers, but there
  still remain many gaps which are due to inefficiencies and lack of certain features, like glyph
  caching.
- **API surface**: I am still in the process of figuring out the right API surfaces for all crates.

I am currently actively working on resolving all of these.

## Crates
As mentioned above, the `hayro` project consists of a number of sub-crates, each of which can be
used individually if desired. These include:
- [`hayro-syntax`](hayro-syntax): Low-level parsing and reading of PDF files.
- [`hayro-interpret`](hayro-interpret): A PDF interpreter emitting commands into an abstract
  `Device`.
- [`hayro`](hayro): Rendering PDF pages into bitmaps.
- [`hayro-svg`](hayro-svg): Converting PDF pages into SVG images.
- [`hayro-jpeg2000`](hayro-jpeg2000): A JPEG2000 image decoder.
- [`hayro-jbig2`](hayro-jbig2): A JBIG2 image decoder.
- [`hayro-ccitt`](hayro-ccitt): A decoder for group 3 and group 4 CCITT-encoded images.
- [`hayro-postscript`](hayro-postscript): A lightweight scanner for a specific subset of PostScript.
- [`hayro-cmap`](hayro-cmap): A parser for CMap files in PDFs.

## Demo
A demo tool can be found at https://laurenzv.github.io/hayro/. Please note that this is just a
quick, vibe-coded PDF viewer, not intended to be used as an actual viewer. Features such as text
selection, search, annotations, and forms are not implemented there and out of scope. It's only
meant as a quick way to test the rendering capabilities and performance of `hayro`.

## Minimum Supported Rust Version (MSRV)
The minimum supported Rust version is **1.92**.
