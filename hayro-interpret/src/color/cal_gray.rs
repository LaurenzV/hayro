use super::{ToRgb, U8Lookup, apply_u8_lookup};
use hayro_syntax::object::Dict;
use hayro_syntax::object::dict::keys::{BLACK_POINT, GAMMA, WHITE_POINT};

#[derive(Debug, Clone)]
pub(crate) struct CalGray {
    white_point: [f32; 3],
    black_point: [f32; 3],
    gamma: f32,
    lookup: U8Lookup,
}

// See <https://github.com/mozilla/pdf.js/blob/06f44916c8936b92f464d337fe3a0a6b2b78d5b4/src/core/colorspace.js#L752>
impl CalGray {
    pub(super) fn new(dict: &Dict<'_>) -> Option<Self> {
        let white_point = dict.get::<[f32; 3]>(WHITE_POINT).unwrap_or([1.0, 1.0, 1.0]);
        let black_point = dict.get::<[f32; 3]>(BLACK_POINT).unwrap_or([0.0, 0.0, 0.0]);
        let gamma = dict.get::<f32>(GAMMA).unwrap_or(1.0);

        Some(Self {
            white_point,
            black_point,
            gamma,
            lookup: U8Lookup::default(),
        })
    }

    fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        for (input, output) in input.iter().zip(output.chunks_exact_mut(3)) {
            let g = self.gamma;
            let (_xw, yw, _zw) = {
                let wp = self.white_point;
                (wp[0], wp[1], wp[2])
            };
            let (_xb, _yb, _zb) = {
                let bp = self.black_point;
                (bp[0], bp[1], bp[2])
            };

            let a = *input as f32 / 255.0;
            let ag = a.powf(g);
            let l = yw * ag;
            let val = (0.0_f32.max(295.8 * l.powf(0.333_333_34) - 40.8) + 0.5) as u8;

            output.copy_from_slice(&[val, val, val]);
        }

        Some(())
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }
}

impl ToRgb for CalGray {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        apply_u8_lookup(input, output, self.u8_lookup()?);

        Some(())
    }
}
