use super::ToRgb;
use super::icc::ICCProfile;
use hayro_syntax::object::Dict;
use hayro_syntax::object::dict::keys::{BLACK_POINT, RANGE, WHITE_POINT};
use moxcms::{ColorProfile, Xyzd};

#[derive(Debug, Clone)]
pub(crate) struct Lab {
    pub(super) range: [f32; 4],
    profile: ICCProfile,
}

impl Lab {
    pub(super) fn new(dict: &Dict<'_>) -> Option<Self> {
        let white_point = dict.get::<[f32; 3]>(WHITE_POINT).unwrap_or([1.0, 1.0, 1.0]);
        // Not sure how this should be used.
        let _black_point = dict.get::<[f32; 3]>(BLACK_POINT).unwrap_or([0.0, 0.0, 0.0]);
        let range = dict
            .get::<[f32; 4]>(RANGE)
            .unwrap_or([-100.0, 100.0, -100.0, 100.0]);

        let mut profile =
            ColorProfile::new_from_slice(include_bytes!("../../assets/LAB.icc")).ok()?;
        profile.white_point = Xyzd::new(
            white_point[0] as f64,
            white_point[1] as f64,
            white_point[2] as f64,
        );

        let profile = ICCProfile::new_from_src_profile(profile, false, 3)?;

        Some(Self { range, profile })
    }
}

impl ToRgb for Lab {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        self.profile.convert(input, output)
    }

    fn convert_in_place(&self, input: &mut [u8]) -> Option<()> {
        self.profile.convert_in_place(input)
    }
}
