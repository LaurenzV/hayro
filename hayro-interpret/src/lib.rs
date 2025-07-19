use kurbo::{Affine, Cap, Join, Point, Shape};
use smallvec::{SmallVec, smallvec};
use std::sync::Arc;

pub mod cache;
pub mod color;
pub mod context;
mod convert;
pub mod device;
pub mod font;
mod interpret;
pub mod mask;
mod paint;
pub mod pattern;
pub mod shading;
mod soft_mask;
mod types;
pub mod util;
pub mod x_object;

use crate::font::FontQuery;

pub use hayro_syntax::*;
pub use interpret::*;
pub use soft_mask::*;
pub use types::*;

pub use paint::{Paint, PaintType};

/// A container for the bytes of a PDF file.
pub type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;
pub type FontResolverFn = Arc<dyn Fn(&FontQuery) -> Option<FontData> + Send + Sync>;
pub type WarningSinkFn = Arc<dyn Fn(InterpreterWarning) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct StrokeProps {
    pub line_width: f32,
    pub line_cap: Cap,
    pub line_join: Join,
    pub miter_limit: f32,
    pub dash_array: SmallVec<[f32; 4]>,
    pub dash_offset: f32,
}

#[derive(Clone, Debug, Copy)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Debug)]
pub struct FillProps {
    pub fill_rule: FillRule,
}
