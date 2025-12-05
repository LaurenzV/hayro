/*!
A crate for interpreting PDF files.

This crate provides an abstraction to interpret the content of a PDF file and render them
into an abstract [`Device`], which clients can implement as needed. This allows you, for
example, to render PDF files to bitmaps (which is what the `hayro` crate does), or other formats
such as SVG.

It should be noted that this crate is still very much in development. Therefore it currently
lacks pretty much any documentation on how to use it. It's current API also only really makes it
useful for rendering to PNG or SVG, though this will be improved upon in the future.

# Examples
See the `examples` folder on the GitHub repository. Apart from that, you can also consult
the implementation of `hayro` and `hayro-svg` to get an idea on how to use this crate.

# Safety
This crate forbids unsafe code via a crate-level attribute.

# Cargo features
This crate has one optional feature:
- `embed-fonts`: PDF processors are required to support 14 predefined fonts that do not need to be
  embedded into a PDF file. If you enable this feature, hayro will embed a (permissively-licensed)
  substitute for each font, so that you don't have to implement your custom font loading logic. This
  will add around ~240KB to your binary.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod cache;
mod context;
mod convert;
mod device;
mod function;
mod interpret;
mod ocg;
mod soft_mask;
mod types;
mod x_object;

pub mod color;
pub mod encode;
pub mod font;
pub mod pattern;
pub mod shading;
pub mod util;

pub use cache::CacheKey;
pub use context::*;
pub use device::*;
pub use function::Function;
pub use hayro_syntax;
pub use hayro_syntax::Pdf;
pub use interpret::*;
pub use soft_mask::*;
pub use types::*;
