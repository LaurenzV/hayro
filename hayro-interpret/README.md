# hayro-interpret

<!-- cargo-rdme start -->

A crate for interpreting PDF files.

This crate provides an abstraction to interpret the content of a PDF file and render them
into an abstract [`Device`], which clients can implement as needed. This allows you, for
example, to render PDF files to bitmaps (which is what the `hayro` crate does), or other formats
such as SVG.

It should be noted that this crate is still very much in development. Therefore it currently
lacks pretty much any documentation on how to use it. It's current API also only really makes it
useful for rendering to PNG or SVG, though this will be improved upon in the future.

<!-- cargo-rdme end -->
