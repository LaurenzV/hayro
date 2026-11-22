use super::ToRgb;
use super::cal_rgb::d65_xyz_to_srgb;
use hayro_syntax::object::Dict;
use hayro_syntax::object::dict::keys::{BLACK_POINT, RANGE, WHITE_POINT};
use moxcms::{Chromaticity, Matrix3f, Vector3f, Xyz, adaption_matrix};

#[derive(Debug, Clone)]
pub(crate) struct Lab {
    pub(super) range: [f32; 4],
    to_d65: Matrix3f,
}

impl Lab {
    pub(super) fn new(dict: &Dict<'_>) -> Option<Self> {
        let white_point = dict.get::<[f32; 3]>(WHITE_POINT).unwrap_or([1.0, 1.0, 1.0]);
        let _black_point = dict.get::<[f32; 3]>(BLACK_POINT).unwrap_or([0.0, 0.0, 0.0]);
        let range = dict
            .get::<[f32; 4]>(RANGE)
            .unwrap_or([-100.0, 100.0, -100.0, 100.0]);

        if white_point[0] <= 0.0
            || white_point[1] != 1.0
            || white_point[2] <= 0.0
            || !(range[0] <= range[1] && range[2] <= range[3])
        {
            return None;
        }
        let source = Xyz::new(white_point[0], white_point[1], white_point[2]);
        let mut to_d65 = adaption_matrix(source, Chromaticity::D65.to_xyz());
        // Lab produces source-relative XYZ, so scale each column before adapting to D65.
        for row in &mut to_d65.v {
            for (coefficient, reference) in row.iter_mut().zip(white_point) {
                *coefficient *= reference;
            }
        }
        to_d65
            .v
            .iter()
            .flatten()
            .all(|n| n.is_finite())
            .then_some(Self { range, to_d65 })
    }

    fn convert_pixel(&self, [l, a, b]: [u8; 3]) -> [u8; 3] {
        let l = f32::from(l) * (100.0 / 255.0);
        let a = (f32::from(a) - 128.0).clamp(self.range[0], self.range[1]);
        let b = (f32::from(b) - 128.0).clamp(self.range[2], self.range[3]);
        let y = (l + 16.0) / 116.0;
        let relative = [y + a / 500.0, y, y - b / 200.0].map(|t| {
            if t > 6.0 / 29.0 {
                t * t * t
            } else {
                (108.0 / 841.0) * (t - 4.0 / 29.0)
            }
        });
        d65_xyz_to_srgb(self.to_d65.mul_vector(Vector3f { v: relative }).v)
    }
}

impl ToRgb for Lab {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        for (src, dst) in input
            .as_chunks::<3>()
            .0
            .iter()
            .zip(output.as_chunks_mut::<3>().0)
        {
            *dst = self.convert_pixel(*src);
        }
        Some(())
    }

    fn convert_in_place(&self, input: &mut [u8]) -> Option<()> {
        for pixel in input.as_chunks_mut::<3>().0 {
            *pixel = self.convert_pixel(*pixel);
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro_syntax::object::FromBytes;

    #[test]
    fn lab_uses_the_source_white_point_for_colour_and_neutrals() {
        let default = Dict::from_bytes(b"<< /WhitePoint [0.9505 1 1.089] >>").unwrap();
        let alternate = Dict::from_bytes(b"<< /WhitePoint [0.75 1 1.4] >>").unwrap();
        let samples = [140, 183, 88, 0, 128, 128, 255, 128, 128];
        for (dict, rgb) in [(default, [190, 92, 202]), (alternate, [168, 108, 201])] {
            let space = Lab::new(&dict).unwrap();
            let mut output = [0_u8; 9];
            space.convert(&samples, &mut output).unwrap();
            assert!(
                output[..3]
                    .iter()
                    .zip(rgb)
                    .all(|(value, expected)| value.abs_diff(expected) <= 2),
                "{output:?}"
            );
            assert_eq!(&output[3..], &[0, 0, 0, 255, 255, 255]);
            let mut inplace = samples;
            space.convert_in_place(&mut inplace).unwrap();
            assert_eq!(output, inplace);
        }
    }

    #[test]
    fn lab_clamps_the_declared_range_and_rejects_invalid_white_points() {
        let dict =
            Dict::from_bytes(b"<< /WhitePoint [0.9505 1 1.089] /Range [-10 15 -20 30] >>").unwrap();
        let space = Lab::new(&dict).unwrap();
        assert_eq!(
            space.convert_pixel([140, 255, 0]),
            space.convert_pixel([140, 143, 108])
        );
        let invalid = Dict::from_bytes(b"<< /WhitePoint [1 0 1] >>").unwrap();
        assert!(Lab::new(&invalid).is_none());
    }
}
