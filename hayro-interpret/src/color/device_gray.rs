use super::{ColorComponent, ToRgb};

#[derive(Debug, Clone)]
pub(crate) struct DeviceGray;

impl ToRgb for DeviceGray {
    fn convert<T: ColorComponent>(&self, input: &[T], output: &mut [u8]) -> Option<()> {
        for (gray, output) in input.iter().zip(output.chunks_exact_mut(3)) {
            let gray = gray.to_u8();
            output.copy_from_slice(&[gray, gray, gray]);
        }

        Some(())
    }
}
