/*!
A crate for interpreting PDF files.

This crate provides an abstraction to interpret the content of a PDF file and render them
into an abstract [`Device`], which clients can implement as needed. This allows you, for
example, to render PDF files to bitmaps (which is what the `hayro-render` crate does), or other formats
such as SVG.

It should be noted that this crate is still very much in development. Therefore it currently
lacks pretty much any documentation on how to use it. It's current API also only really makes it
useful for rendering to PNG or SVG, though this will be improved upon in the future.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod cache;
mod context;
mod convert;
mod device;
mod interpret;
mod soft_mask;
mod types;
mod x_object;

pub mod color;
pub mod font;
pub mod pattern;
pub mod shading;
pub mod util;

pub use context::*;
pub use device::*;
pub use hayro_syntax;
pub use hayro_syntax::Pdf;
pub use interpret::*;
pub use soft_mask::*;
pub use types::*;
