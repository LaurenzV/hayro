use super::{ColorComponents, ColorSpace, ToRgb};

#[derive(Debug, Clone)]
pub(crate) struct Pattern(ColorSpace);

impl Pattern {
    pub(super) fn new(color_space: ColorSpace) -> Self {
        Self(color_space)
    }

    pub(super) fn color_space(&self) -> ColorSpace {
        self.0.clone()
    }

    pub(super) fn initial_color(&self) -> ColorComponents {
        self.0.initial_color()
    }

    pub(super) fn num_components(&self) -> u8 {
        self.0.num_components()
    }
}

impl ToRgb for Pattern {
    fn convert_f32(&self, input: &[f32], output: &mut [u8], manual_scale: bool) -> Option<()> {
        self.0.convert_f32(input, output, manual_scale)
    }

    fn supports_u8(&self) -> bool {
        self.0.supports_u8()
    }

    fn convert_u8(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        self.0.convert_u8(input, output)
    }
}
