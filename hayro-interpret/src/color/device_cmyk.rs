use super::ToRgb;
use super::icc::ICCProfile;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub(crate) struct DeviceCmyk;

impl ToRgb for DeviceCmyk {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        CMYK_TRANSFORM.convert(input, output)
    }
}

static CMYK_TRANSFORM: LazyLock<ICCProfile> = LazyLock::new(|| {
    ICCProfile::new(
        include_bytes!("../../assets/CGATS001Compat-v2-micro.icc"),
        4,
    )
    .unwrap()
});
