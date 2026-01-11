// Note that these polyfills can be very imprecise, but hopefully good enough
// for the vast majority of cases.

#[inline(always)]
pub(crate) fn round_f32(x: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        x.round()
    }
    #[cfg(not(feature = "std"))]
    {
        if x >= 0.0 {
            floor_f32(x + 0.5)
        } else {
            -floor_f32(-x + 0.5)
        }
    }
}

#[inline(always)]
#[cfg(not(feature = "std"))]
fn floor_f32(x: f32) -> f32 {
    let xi = x as i32;
    let xf = xi as f32;
    if x < xf { xf - 1.0 } else { xf }
}

#[inline(always)]
pub(crate) fn trunc_f64(x: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        x.trunc()
    }
    #[cfg(not(feature = "std"))]
    {
        x as i64 as f64
    }
}

#[inline(always)]
pub(crate) fn fract_f64(x: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        x.fract()
    }
    #[cfg(not(feature = "std"))]
    {
        x - trunc_f64(x)
    }
}
