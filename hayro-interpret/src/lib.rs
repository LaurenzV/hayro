/*!
A crate for interpreting PDF files.

This crate provides an abstraction to interpret the content of a PDF file and render them
into an abstract `Device`, which can be implemented by clients as needed. This allows you to for
example render PDF files to bitmaps (which is what the `hayro-render` crate does), or other formats
such as SVG.

It should be noted that this crate is still very much in development. Therefore it currently
lacks pretty much any documentation on how to use it. It's current API also only really makes it
useful for rendering to PNG or SVG, though this will be improved upon in the future.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use kurbo::{Affine, Cap, Join, Point, Shape};
use smallvec::{SmallVec, smallvec};
use std::sync::Arc;

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

use crate::font::FontQuery;

pub use context::*;
pub use device::*;
pub use hayro_syntax::*;
pub use interpret::*;
pub use soft_mask::*;
pub use types::*;

/// A container for the bytes of a PDF file.
pub type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;
/// A callback function for resolving font queries.
pub type FontResolverFn = Arc<dyn Fn(&FontQuery) -> Option<FontData> + Send + Sync>;
/// A callback function for resolving warnings during interpretation.
pub type WarningSinkFn = Arc<dyn Fn(InterpreterWarning) + Send + Sync>;

/// Stroke properties.
#[derive(Clone, Debug)]
pub struct StrokeProps {
    /// The line width.
    pub line_width: f32,
    /// The line cap.
    pub line_cap: Cap,
    /// The line join.
    pub line_join: Join,
    /// The miter limit.
    pub miter_limit: f32,
    /// The dash array.
    pub dash_array: SmallVec<[f32; 4]>,
    /// The dash offset.
    pub dash_offset: f32,
}

/// A fill rule.
#[derive(Clone, Debug, Copy)]
pub enum FillRule {
    /// Non-zero filling.
    NonZero,
    /// Even-odd filling.
    EvenOdd,
}

/// Fill properties.
#[derive(Clone, Debug)]
pub struct FillProps {
    /// The fill rule.
    pub fill_rule: FillRule,
}
