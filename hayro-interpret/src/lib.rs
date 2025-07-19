use kurbo::{Affine, Cap, Join, Point, Shape};
use smallvec::{SmallVec, smallvec};
use std::sync::Arc;

mod convert;
mod interpret;
mod paint;
mod soft_mask;
mod types;
mod x_object;

pub mod cache;
pub mod color;
pub mod context;
pub mod device;
pub mod font;
pub mod mask;
pub mod pattern;
pub mod shading;
pub mod util;

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
