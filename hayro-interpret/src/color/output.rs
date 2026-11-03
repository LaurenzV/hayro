use moxcms::{
    ColorProfile, DataColorSpace, Layout, RenderingIntent, Transform8BitExecutor, TransformOptions,
};
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

/// An ICC profile for CMYK output.
///
/// [`ColorSpace::to_cmyk`](super::ColorSpace::to_cmyk) converts colors to CMYK through it,
/// from sRGB and with the relative colorimetric rendering intent, the default intent of PDF.
#[derive(Clone)]
pub struct CmykProfile(Arc<Transform8BitExecutor>);

impl Debug for CmykProfile {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "CmykProfile {{..}}")
    }
}

impl CmykProfile {
    /// Create a profile from the data of an ICC profile.
    ///
    /// Returns `None` if the data is not a CMYK profile that colors can be converted to, for
    /// example a profile without a `B2A` table.
    pub fn new(data: &[u8]) -> Option<Self> {
        let profile = ColorProfile::new_from_slice(data).ok()?;

        if profile.color_space != DataColorSpace::Cmyk {
            return None;
        }

        let options = TransformOptions {
            rendering_intent: RenderingIntent::RelativeColorimetric,
            ..TransformOptions::default()
        };
        let transform = ColorProfile::new_srgb()
            .create_transform_8bit(Layout::Rgb, &profile, Layout::Rgba, options)
            .ok()?;

        Some(Self(transform))
    }
}

/// Convert an sRGB color to CMYK, as [`rgb_to_cmyk`] does.
pub(crate) fn srgb_to_cmyk(rgb: [f32; 3], profile: Option<&CmykProfile>) -> [f32; 4] {
    let mut cmyk = [0; 4];
    let rgb = rgb.map(|value| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);

    if rgb_to_cmyk(profile, &rgb, &mut cmyk).is_none() {
        return [0.0, 0.0, 0.0, 1.0];
    }

    cmyk.map(|value| value as f32 / 255.0)
}

/// Convert sRGB8 values to CMYK8 values through `profile`.
///
/// Without a profile, this is the conversion from `DeviceRGB` to `DeviceCMYK` of the PDF
/// specification, with black generation and undercolor removal that move all of the gray
/// component into black.
pub(super) fn rgb_to_cmyk(
    profile: Option<&CmykProfile>,
    input: &[u8],
    output: &mut [u8],
) -> Option<()> {
    if let Some(profile) = profile {
        return profile.0.transform(input, output).ok();
    }

    for (rgb, cmyk) in input
        .as_chunks::<3>()
        .0
        .iter()
        .zip(output.as_chunks_mut::<4>().0)
    {
        let [c, m, y] = rgb.map(|value| 255 - value);
        let k = c.min(m).min(y);
        *cmyk = [c - k, m - k, y - k, k];
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use super::CmykProfile;
    use crate::cache::Cache;
    use crate::color::ColorSpace;
    use hayro_syntax::object::{FromBytes, Object};
    use moxcms::{
        ColorProfile, DataColorSpace, LutDataType, LutStore, LutType, LutWarehouse, Matrix3d,
        ProfileClass,
    };

    /// A minimal CMYK output profile.
    fn test_profile() -> CmykProfile {
        let identity =
            |channels: usize| LutStore::Store8((0..channels).flat_map(|_| 0..=u8::MAX).collect());
        let lut = LutWarehouse::Lut(LutDataType {
            num_input_channels: 3,
            num_output_channels: 4,
            num_clut_grid_points: 2,
            matrix: Matrix3d::IDENTITY,
            num_input_table_entries: 256,
            num_output_table_entries: 256,
            input_table: identity(3),
            clut_table: LutStore::Store8([0, 255, 0, 255].repeat(8)),
            output_table: identity(4),
            lut_type: LutType::Lut8,
        });
        let mut profile = ColorProfile::default();
        profile.profile_class = ProfileClass::OutputDevice;
        profile.color_space = DataColorSpace::Cmyk;
        profile.pcs = DataColorSpace::Lab;
        profile.lut_b_to_a_perceptual = Some(lut.clone());
        profile.lut_b_to_a_colorimetric = Some(lut);

        CmykProfile::new(&profile.encode().unwrap()).unwrap()
    }

    fn to_cmyk(color_space: &[u8], components: &[f32], profile: Option<&CmykProfile>) -> [u8; 5] {
        let object = Object::from_bytes(color_space).unwrap();
        ColorSpace::new(object, &Cache::new())
            .unwrap()
            .to_cmyk(components, 1.0, profile)
            .map(|value| (value * 255.0 + 0.5) as u8)
    }

    #[test]
    fn device_colors_keep_their_inks() {
        let profile = test_profile();

        for profile in [None, Some(&profile)] {
            let cmyk = to_cmyk(b"/DeviceCMYK", &[0.2, 0.4, 0.6, 0.8], profile);
            assert_eq!(cmyk, [51, 102, 153, 204, 255]);
            assert_eq!(
                to_cmyk(b"/DeviceGray", &[0.2], profile),
                [0, 0, 0, 204, 255]
            );
        }
    }

    #[test]
    fn other_colors_without_profile() {
        assert_eq!(
            to_cmyk(b"/DeviceRGB", &[1.0, 0.0, 0.0], None),
            [0, 255, 255, 0, 255]
        );
        assert_eq!(
            to_cmyk(b"/DeviceRGB", &[0.2, 0.2, 0.2], None),
            [0, 0, 0, 204, 255]
        );
        assert_eq!(
            to_cmyk(b"/DeviceRGB", &[0.2, 0.6, 1.0], None),
            [204, 102, 0, 0, 255]
        );
    }

    /// Convert a color to sRGB and from there to CMYK through `profile` directly.
    fn through_profile(color_space: &[u8], components: &[f32], profile: &CmykProfile) -> [u8; 5] {
        let object = Object::from_bytes(color_space).unwrap();
        let rgba = ColorSpace::new(object, &Cache::new())
            .unwrap()
            .to_rgba(components, 1.0)
            .to_rgba8();
        let mut cmyk = [0; 4];
        profile.0.transform(&rgba[..3], &mut cmyk).unwrap();

        [cmyk[0], cmyk[1], cmyk[2], cmyk[3], 255]
    }

    #[test]
    fn other_colors_with_profile() {
        let profile = test_profile();
        let colors: [(&[u8], &[f32]); 4] = [
            (b"/DeviceRGB", &[1.0, 0.5, 0.0]),
            (
                b"[/Lab << /WhitePoint [0.9505 1 1.089] >>]",
                &[50.0, 20.0, -30.0],
            ),
            (b"[/CalGray << /WhitePoint [0.9505 1 1.089] >>]", &[0.5]),
            (
                b"[/CalRGB << /WhitePoint [0.9505 1 1.089] >>]",
                &[0.2, 0.4, 0.6],
            ),
        ];

        for (color_space, components) in colors {
            let cmyk = to_cmyk(color_space, components, Some(&profile));
            assert_eq!(cmyk, through_profile(color_space, components, &profile));
            assert_ne!(cmyk, to_cmyk(color_space, components, None));
        }
    }

    #[test]
    fn special_colors_use_their_base() {
        let profile = Some(test_profile());
        let profile = profile.as_ref();

        let spot = b"[/Separation /Spot /DeviceCMYK
            << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.5 1 0] /N 1 >>]";
        assert_eq!(to_cmyk(spot, &[1.0], profile), [0, 128, 255, 0, 255]);
        assert_eq!(to_cmyk(spot, &[0.5], profile), [0, 64, 128, 0, 255]);
        // The None colorant paints nothing.
        let none = b"[/Separation /None /DeviceCMYK
            << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0 0 1] /N 1 >>]";
        assert_eq!(to_cmyk(none, &[1.0], profile), [0, 0, 0, 255, 0]);
        let red = b"[/Separation /Red /DeviceRGB
            << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>]";
        assert_eq!(to_cmyk(red, &[1.0], None), [0, 255, 255, 0, 255]);
        let device_red = to_cmyk(b"/DeviceRGB", &[1.0, 0.0, 0.0], profile);
        assert_eq!(to_cmyk(red, &[1.0], profile), device_red);
        let indexed = b"[/Indexed /DeviceCMYK 1 <00000000 00FF8000>]";
        assert_eq!(to_cmyk(indexed, &[1.0], profile), [0, 255, 128, 0, 255]);
    }

    #[test]
    fn profiles_without_cmyk_output() {
        // hayro's CMYK profile only converts from CMYK.
        let input_only = include_bytes!("../../assets/CGATS001Compat-v2-micro.icc");
        assert!(CmykProfile::new(input_only).is_none());
        assert!(CmykProfile::new(&ColorProfile::new_srgb().encode().unwrap()).is_none());
        assert!(CmykProfile::new(b"not a profile").is_none());
    }
}
