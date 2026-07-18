use super::{ToRgb, f32_to_u8};

#[derive(Debug, Clone)]
pub(crate) struct DeviceGray;

impl ToRgb for DeviceGray {
    fn convert_f32(&self, input: &[f32], output: &mut [u8], _: bool) -> Option<()> {
        for (gray, output) in input.iter().zip(output.chunks_exact_mut(3)) {
            let gray = f32_to_u8(*gray);
            output.copy_from_slice(&[gray, gray, gray]);
        }

        Some(())
    }

    fn supports_u8(&self) -> bool {
        true
    }

    fn convert_u8(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        for (input, output) in input.iter().zip(output.chunks_exact_mut(3)) {
            output.copy_from_slice(&[*input, *input, *input]);
        }

        Some(())
    }
}
