#[cfg_attr(feature = "simd", allow(dead_code))]
#[inline(always)]
pub(crate) fn mul_add(a: f32, b: f32, c: f32) -> f32 {
    #[cfg(any(
        all(
            any(target_arch = "x86", target_arch = "x86_64"),
            target_feature = "fma"
        ),
        all(target_arch = "aarch64", target_feature = "neon")
    ))]
    {
        f32::mul_add(a, b, c)
    }
    #[cfg(not(any(
        all(
            any(target_arch = "x86", target_arch = "x86_64"),
            target_feature = "fma"
        ),
        all(target_arch = "aarch64", target_feature = "neon")
    )))]
    {
        a * b + c
    }
}
