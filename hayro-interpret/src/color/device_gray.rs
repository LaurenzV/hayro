use super::{ToLuma, ToRgb};

#[derive(Debug, Clone)]
pub(crate) struct DeviceGray;

impl ToRgb for DeviceGray {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        for (gray, output) in input.iter().zip(output.chunks_exact_mut(3)) {
            output.copy_from_slice(&[*gray, *gray, *gray]);
        }

        Some(())
    }
}

impl ToLuma for DeviceGray {
    fn to_luma(&self, _input: &mut [u8]) -> Option<()> {
        Some(())
    }
}
