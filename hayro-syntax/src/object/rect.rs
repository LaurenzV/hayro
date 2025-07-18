//! Rectangles.

use crate::object::Array;
use crate::object::{Object, ObjectLike};
use crate::reader::{Readable, Reader, ReaderContext};

pub use kurbo::Rect;

impl Readable<'_> for Rect {
    fn read(r: &mut Reader<'_>, _: ReaderContext) -> Option<Self> {
        let arr = r.read_without_context::<Array>()?;
        from_arr(&arr)
    }
}

fn from_arr(array: &Array) -> Option<Rect> {
    let mut iter = array.iter::<f32>();

    Some(Rect::new(
        iter.next()? as f64,
        iter.next()? as f64,
        iter.next()? as f64,
        iter.next()? as f64,
    ))
}

impl TryFrom<Object<'_>> for Rect {
    type Error = ();

    fn try_from(value: Object<'_>) -> Result<Self, Self::Error> {
        match value {
            Object::Array(arr) => from_arr(&arr).ok_or(()),
            _ => Err(()),
        }
    }
}

impl ObjectLike<'_> for Rect {}
