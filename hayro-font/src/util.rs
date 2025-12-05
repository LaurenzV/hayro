/// Just like `TryFrom`<N>, but for numeric types not supported by the Rust's std.
pub(crate) trait TryNumFrom<T>: Sized {
    /// Casts between numeric types.
    fn try_num_from(_: T) -> Option<Self>;
}

impl TryNumFrom<f32> for u8 {
    #[inline]
    fn try_num_from(v: f32) -> Option<Self> {
        i32::try_num_from(v).and_then(|v| Self::try_from(v).ok())
    }
}

impl TryNumFrom<f32> for i16 {
    #[inline]
    fn try_num_from(v: f32) -> Option<Self> {
        i32::try_num_from(v).and_then(|v| Self::try_from(v).ok())
    }
}

impl TryNumFrom<f32> for u16 {
    #[inline]
    fn try_num_from(v: f32) -> Option<Self> {
        i32::try_num_from(v).and_then(|v| Self::try_from(v).ok())
    }
}

#[allow(clippy::manual_range_contains)]
impl TryNumFrom<f32> for i32 {
    #[inline]
    fn try_num_from(v: f32) -> Option<Self> {
        // Based on https://github.com/rust-num/num-traits/blob/master/src/cast.rs

        // Float as int truncates toward zero, so we want to allow values
        // in the exclusive range `(MIN-1, MAX+1)`.

        // We can't represent `MIN-1` exactly, but there's no fractional part
        // at this magnitude, so we can just use a `MIN` inclusive boundary.
        const MIN: f32 = i32::MIN as f32;
        // We can't represent `MAX` exactly, but it will round up to exactly
        // `MAX+1` (a power of two) when we cast it.
        const MAX_P1: f32 = i32::MAX as f32;
        if v >= MIN && v < MAX_P1 {
            Some(v as Self)
        } else {
            None
        }
    }
}
