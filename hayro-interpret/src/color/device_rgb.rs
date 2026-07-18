use super::{ToRgb, f32_to_u8};

#[derive(Debug, Clone)]
pub(crate) struct DeviceRgb;

impl ToRgb for DeviceRgb {
    fn convert_f32(&self, input: &[f32], output: &mut [u8], _: bool) -> Option<()> {
        for (input, output) in input.iter().copied().zip(output) {
            *output = f32_to_u8(input);
        }

        Some(())
    }

    fn supports_u8(&self) -> bool {
        true
    }

    fn convert_u8(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        for (input, output) in input.iter().zip(output.iter_mut()) {
            *output = *input;
        }

        Some(())
    }
}
