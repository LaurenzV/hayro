# hayro

[![Crates.io](https://img.shields.io/crates/v/hayro.svg)](https://crates.io/crates/hayro)
[![Documentation](https://docs.rs/hayro/badge.svg)](https://docs.rs/hayro)

An experimental, work-in-progress PDF interpreter and renderer.

`hayro` is a Rust crate with a simple task: It allows you to interpret one or many pages of a PDF file to for exmaple convert them into PNG or SVG files. This is a difficult task, as the PDF specification is _huge_ and contains many features. In addition to that, there are millions of PDF files out there with many edge cases, so a solid PDF renderer should be able to handle those as well as possible.

This is not the first attempt at writing a PDF renderer in Rust, but, to the best of my knowledge, this is currently the most feature-complete library. There are still a few important features that `hayro` currently doesn't support, including loading password-protected PDFs and rendering blend modes and knockout transparency groups. With that said, we _do_ support most of the main features, meaning that you should be able to render the "average" PDF file without encountering any issues. This statement is underpinned by the fact that `hayro` is able to handle the 1000+ PDFs in our test suite, which to a large part have been scraped from the `PDFBOX` and `pdf.js` test regression suites.

But, this crate is still in a very development stage, and there are many issues remaining that need to be addressed, including performance, which has not been a focus at all so far but will become a priority in the future.

## Crates
While the main goal of `hayro` is rendering PDF files, the `hayro` project actually encompasses a number of different crates which can in theory used independently. These include:
- [`hayro-syntax`](hayro-syntax): A crate for low-level reading and parsing of PDF files.
- [`hayro-interpret`](hayro-interpret): A crate for interpreting PDF pages and rendering them into an abstract `Device` implementation.
- [`hayro`](hayro): A crate for rendering PDF files into bitmaps.
- [`hayro-svg`](hayro-svg): A crate for converting PDF pages into SVG files.
- [`hayro-font`](hayro-font): A crate for parsing Type1 and CFF fonts.

## Demo
A demo tool can be found at https://laurenzv.github.io/hayro/. Please note that this is not intended to be a PDF viewer application: It misses many important features like zooming, selecting text and important optimizations for improving the user experience. It's really just meant as a quick way to test the rendering capabilities of `hayro`. Note that PDFs with embedded JPEG2000 images will not display correctly in this web demo.

## License
All crates in this repository are available under the Apache 2.0 license.