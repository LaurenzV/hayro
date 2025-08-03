use crate::encode::{EncodeExt, EncodedPaint, Shader};
use crate::fine::Sampler;
use crate::paint::{IndexedPaint, Paint};
use hayro_interpret::encode::EncodedShadingPattern;
use hayro_interpret::pattern::ShadingPattern;
use kurbo::Point;

impl Sampler for EncodedShadingPattern {
    fn interpolate(&self) -> bool {
        false
    }

    fn sample_impl(&self, pos: Point) -> [f32; 4] {
        Self::sample(self, pos)
    }
}

impl EncodeExt for ShadingPattern {
    fn encode_into(&self, paints: &mut Vec<EncodedPaint>) -> Paint {
        let idx = paints.len();

        let encoded = self.encode();

        let shader = Shader::<EncodedShadingPattern>::new(encoded);
        paints.push(EncodedPaint::Shading(shader));
        Paint::Indexed(IndexedPaint::new(idx))
    }
}
