use crate::encode::{Buffer, EncodeExt, EncodedPaint, Shader};
use crate::fine::Sampler;
use crate::paint::{Image, IndexedPaint, Paint};
use kurbo::Point;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct EncodedImage {
    pub(crate) buffer: Arc<Buffer<4>>,
    pub(crate) interpolate: bool,
    pub(crate) is_pattern: bool,
}

impl EncodeExt for Image {
    fn encode_into(&self, paints: &mut Vec<EncodedPaint>) -> Paint {
        let idx = paints.len();

        let encoded = EncodedImage {
            buffer: self.buffer.clone(),
            interpolate: self.interpolate,
            is_pattern: self.is_pattern,
        };

        let shader = Shader::<EncodedImage>::new(self.transform.inverse(), encoded);

        if self.is_stencil {
            paints.push(EncodedPaint::Mask(shader));
        } else {
            paints.push(EncodedPaint::Image(shader));
        }

        Paint::Indexed(IndexedPaint::new(idx))
    }
}

impl Sampler for EncodedImage {
    fn interpolate(&self) -> bool {
        self.interpolate
    }

    fn sample_impl(&self, mut pos: Point) -> [f32; 4] {
        if self.is_pattern {
            let extend = |val: f64, max: f64| val - (val / max).floor() * max;
            pos.x = extend(pos.x, self.buffer.width as f64);
            pos.y = extend(pos.y, self.buffer.height as f64);
        } else {
            pos.x = pos.x.clamp(0.0, self.buffer.width as f64 - 1.0);
            pos.y = pos.y.clamp(0.0, self.buffer.height as f64 - 1.0);
        }

        self.buffer.sample(pos)
    }
}
