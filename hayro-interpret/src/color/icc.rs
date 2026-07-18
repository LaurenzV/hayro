use super::ToRgb;
use moxcms::{
    ColorProfile, DataColorSpace, Layout, Transform8BitExecutor, TransformF32Executor,
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
    transform_f32: OnceLock<Arc<TransformF32Executor>>,
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
        let is_lab = src_profile.color_space == DataColorSpace::Lab;

        Self::new_from_src_profile(src_profile, is_srgb, is_lab, number_components)
    }

    pub(super) fn new_from_src_profile(
        src_profile: ColorProfile,
        is_srgb: bool,
        is_lab: bool,
        number_components: usize,
    ) -> Option<Self> {
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
            transform_f32: OnceLock::new(),
        })))
    }

    pub(super) fn number_components(&self) -> usize {
        self.0.number_components
    }

    pub(super) fn is_srgb(&self) -> bool {
        self.0.is_srgb
    }

    fn is_lab(&self) -> bool {
        self.0.is_lab
    }

    fn transform_u8(&self) -> &Arc<Transform8BitExecutor> {
        &self.0.transform_u8
    }

    fn transform_f32(&self) -> &Arc<TransformF32Executor> {
        // From my benchmarking, creating the f32 transforms is usually much
        // more expensive than u8. Therefore, we only create it lazily when
        // really needed.
        self.0.transform_f32.get_or_init(|| {
            let dest_profile = ColorProfile::new_srgb();
            self.0
                .src_profile
                .clone()
                .create_transform_f32(
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
    fn convert_f32(&self, input: &[f32], output: &mut [u8], _: bool) -> Option<()> {
        let mut temp = vec![0.0_f32; output.len()];

        if self.is_lab() {
            // moxcms expects normalized values.
            let scaled = input
                .chunks_exact(3)
                .flat_map(|i| {
                    [
                        i[0] * (1.0 / 100.0),
                        (i[1] + 128.0) * (1.0 / 255.0),
                        (i[2] + 128.0) * (1.0 / 255.0),
                    ]
                })
                .collect::<Vec<_>>();
            self.transform_f32().transform(&scaled, &mut temp).ok()?;
        } else {
            self.transform_f32().transform(input, &mut temp).ok()?;
        };

        for (input, output) in temp.iter().zip(output.iter_mut()) {
            *output = (input * 255.0 + 0.5) as u8;
        }

        Some(())
    }

    fn supports_u8(&self) -> bool {
        true
    }

    fn convert_u8(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        if self.is_srgb() {
            output.copy_from_slice(input);
        } else {
            self.transform_u8().transform(input, output).ok()?;
        }

        Some(())
    }
}
