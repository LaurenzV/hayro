use super::ToRgb;

#[derive(Debug, Clone)]
pub(crate) struct DeviceRgb;

impl ToRgb for DeviceRgb {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        output.copy_from_slice(input);

        Some(())
    }

    fn convert_in_place(&self, _input: &mut [u8]) -> Option<()> {
        Some(())
    }
}
