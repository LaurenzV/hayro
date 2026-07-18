use super::icc::ICCProfile;
use super::{ToRgb, f32_to_u8};
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub(crate) struct DeviceCmyk;

impl ToRgb for DeviceCmyk {
    fn convert_f32(&self, input: &[f32], output: &mut [u8], _: bool) -> Option<()> {
        if input.len() == 4 {
            let converted = [
                f32_to_u8(input[0]),
                f32_to_u8(input[1]),
                f32_to_u8(input[2]),
                f32_to_u8(input[3]),
            ];
            CMYK_TRANSFORM.convert_u8(&converted, output)
        } else {
            let converted = input.iter().copied().map(f32_to_u8).collect::<Vec<_>>();
            CMYK_TRANSFORM.convert_u8(&converted, output)
        }
    }

    fn supports_u8(&self) -> bool {
        true
    }

    fn convert_u8(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        CMYK_TRANSFORM.convert_u8(input, output)
    }
}

static CMYK_TRANSFORM: LazyLock<ICCProfile> = LazyLock::new(|| {
    ICCProfile::new(
        include_bytes!("../../assets/CGATS001Compat-v2-micro.icc"),
        4,
    )
    .unwrap()
});
