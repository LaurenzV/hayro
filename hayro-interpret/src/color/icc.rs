use super::ToRgb;
use moxcms::{
    ColorProfile, DataColorSpace, InPlaceTransformExecutor, Layout, Transform8BitExecutor,
    TransformOptions,
};
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, OnceLock};

struct ICCColorRepr {
    number_components: usize,
    is_srgb: bool,
    is_lab: bool,
    src_profile: Option<ColorProfile>,
    src_layout: Layout,
    transform_u8: OnceLock<Option<Arc<Transform8BitExecutor>>>,
    transform_in_place_u8: Option<Arc<dyn InPlaceTransformExecutor<u8> + Send + Sync>>,
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
        let transform_in_place_u8 = if src_layout == Layout::Rgb {
            src_profile
                .create_in_place_transform_8bit(
                    src_layout,
                    &dest_profile,
                    TransformOptions::default(),
                )
                .ok()
        } else {
            None
        };
        let (src_profile, transform_u8) = if transform_in_place_u8.is_some() {
            (Some(src_profile), OnceLock::new())
        } else {
            let transform_u8 = src_profile
                .create_transform_8bit(
                    src_layout,
                    &dest_profile,
                    Layout::Rgb,
                    TransformOptions::default(),
                )
                .ok()?;
            (None, OnceLock::from(Some(transform_u8)))
        };

        Some(Self(Arc::new(ICCColorRepr {
            number_components,
            is_srgb,
            is_lab,
            src_profile,
            src_layout,
            transform_u8,
            transform_in_place_u8,
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

    fn transform_u8(&self) -> Option<&Arc<Transform8BitExecutor>> {
        self.0
            .transform_u8
            .get_or_init(|| {
                self.0
                    .src_profile
                    .as_ref()?
                    .create_transform_8bit(
                        self.0.src_layout,
                        &ColorProfile::new_srgb(),
                        Layout::Rgb,
                        TransformOptions::default(),
                    )
                    .ok()
            })
            .as_ref()
    }

    fn transform_in_place_u8(
        &self,
    ) -> Option<&Arc<dyn InPlaceTransformExecutor<u8> + Send + Sync>> {
        self.0.transform_in_place_u8.as_ref()
    }
}

impl ToRgb for ICCProfile {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        self.transform_u8()?.transform(input, output).ok()?;

        Some(())
    }

    fn convert_in_place(&self, input: &mut [u8]) -> Option<()> {
        if !self.is_srgb() {
            self.transform_in_place_u8()?.transform(input).ok()?;
        }

        Some(())
    }
}
