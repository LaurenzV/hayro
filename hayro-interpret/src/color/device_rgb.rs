use super::{ColorComponent, ToRgb};

#[derive(Debug, Clone)]
pub(crate) struct DeviceRgb;

impl ToRgb for DeviceRgb {
    fn convert<T: ColorComponent>(&self, input: &[T], output: &mut [u8]) -> Option<()> {
        // TODO: This should never be called with u8.
        for (input, output) in input.iter().zip(output) {
            *output = input.to_u8();
        }

        Some(())
    }
}
