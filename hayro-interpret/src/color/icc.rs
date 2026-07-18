use super::{ColorComponent, ColorComponentSlice, ToRgb};
use moxcms::{
    ColorProfile, DataColorSpace, Layout, Transform8BitExecutor, Transform16BitExecutor,
    TransformOptions,
};
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, OnceLock};

struct ICCColorRepr {
    src_profile: ColorProfile,
    src_layout: Layout,
    number_components: usize,
    is_srgb: bool,
    is_lab: bool,
    transform_u8: Arc<Transform8BitExecutor>,
    transform_u16: OnceLock<Arc<Transform16BitExecutor>>,
}

#[derive(Clone)]
pub(crate) struct ICCProfile(Arc<ICCColorRepr>);

impl Debug for ICCProfile {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ICCColor {{..}}")
    }
}

impl ICCProfile {
    pub(super) fn new(profile: &[u8], number_components: usize) -> Option<Self> {
        let src_profile = ColorProfile::new_from_slice(profile).ok()?;

        const SRGB_MARKER: &[u8] = b"sRGB";

        let is_srgb = profile
            .get(52..56)
            .map(|device_model| device_model == SRGB_MARKER)
            .unwrap_or(false);
        Self::new_from_src_profile(src_profile, is_srgb, number_components)
    }

    pub(super) fn new_from_src_profile(
        src_profile: ColorProfile,
        is_srgb: bool,
        number_components: usize,
    ) -> Option<Self> {
        let is_lab = src_profile.color_space == DataColorSpace::Lab;
        let src_layout = match number_components {
            1 => Layout::Gray,
            3 => Layout::Rgb,
            4 => Layout::Rgba,
            _ => {
                warn!("unsupported number of components {number_components} for ICC profile");

                return None;
            }
        };

        let dest_profile = ColorProfile::new_srgb();
        let transform_u8 = src_profile
            .clone()
            .create_transform_8bit(
                src_layout,
                &dest_profile,
                Layout::Rgb,
                TransformOptions::default(),
            )
            .ok()?;

        Some(Self(Arc::new(ICCColorRepr {
            src_profile,
            src_layout,
            number_components,
            is_srgb,
            is_lab,
            transform_u8,
            transform_u16: OnceLock::new(),
        })))
    }

    pub(super) fn number_components(&self) -> usize {
        self.0.number_components
    }

    pub(super) fn is_srgb(&self) -> bool {
        self.0.is_srgb
    }

    pub(super) fn is_lab(&self) -> bool {
        self.0.is_lab
    }

    fn transform_u8(&self) -> &Arc<Transform8BitExecutor> {
        &self.0.transform_u8
    }

    fn transform_u16(&self) -> &Arc<Transform16BitExecutor> {
        // Most images are 8-bit, so only construct this transform when needed.
        self.0.transform_u16.get_or_init(|| {
            let dest_profile = ColorProfile::new_srgb();
            self.0
                .src_profile
                .clone()
                .create_transform_16bit(
                    self.0.src_layout,
                    &dest_profile,
                    Layout::Rgb,
                    TransformOptions::default(),
                )
                // Since the u8 version was valid, hopefully this should never panic?
                .unwrap()
        })
    }
}

impl ToRgb for ICCProfile {
    fn convert<T: ColorComponent>(&self, input: &[T], output: &mut [u8]) -> Option<()> {
        match T::as_slice(input) {
            ColorComponentSlice::U8(input) => {
                // TODO: Change it so we don't need to copy if it's a no-op.
                if self.is_srgb() {
                    output.copy_from_slice(input);
                } else {
                    self.transform_u8().transform(input, output).ok()?;
                }

                Some(())
            }
            ColorComponentSlice::U16(input) => {
                if self.is_srgb() {
                    for (input, output) in input.iter().zip(output) {
                        *output = input.to_u8();
                    }

                    return Some(());
                }

                // TODO: Improve this.
                const BLOCK_PIXELS: usize = 1024;
                let input_block_len = self.number_components() * BLOCK_PIXELS;
                let mut converted = Vec::with_capacity(BLOCK_PIXELS * 3);
                let mut output_offset = 0;

                for input in input.chunks(input_block_len) {
                    let pixels = input.len() / self.number_components();
                    converted.clear();
                    converted.resize(pixels * 3, 0);
                    self.transform_u16().transform(input, &mut converted).ok()?;

                    let output = output.get_mut(output_offset..output_offset + converted.len())?;
                    for (input, output) in converted.iter().zip(output) {
                        *output = input.to_u8();
                    }
                    output_offset += converted.len();
                }

                Some(())
            }
        }
    }
}
