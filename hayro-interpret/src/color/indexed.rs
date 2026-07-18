use super::{ColorSpace, ToRgb};
use crate::cache::Cache;
use hayro_syntax::object::{self, Array, Name, Object, Stream};
use std::borrow::Cow;

#[derive(Debug, Clone)]
pub(crate) struct Indexed {
    values: Vec<Vec<f32>>,
    hival: u8,
    base: Box<ColorSpace>,
}

impl Indexed {
    pub(super) fn new(array: &Array<'_>, cache: &Cache) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip name
        let _ = iter.next::<Name<'_>>()?;
        let base_color_space = ColorSpace::new(iter.next::<Object<'_>>()?, cache)?;
        let hival = iter.next::<u32>()?.min(u8::MAX as u32) as u8;

        let values = {
            let data = iter
                .next::<Stream<'_>>()
                .and_then(|s| s.decoded().ok())
                .or_else(|| {
                    iter.next::<object::String<'_>>()
                        .map(|s| Cow::Owned(s.to_vec()))
                })?;

            let num_components = base_color_space.num_components();

            let mut byte_iter = data.iter().copied();

            let mut vals = vec![];
            for _ in 0..=hival {
                let mut temp = vec![];

                for _ in 0..num_components {
                    temp.push(byte_iter.next()? as f32 / 255.0);
                }

                vals.push(temp);
            }

            vals
        };

        Some(Self {
            values,
            hival,
            base: Box::new(base_color_space),
        })
    }
}

impl ToRgb for Indexed {
    fn convert_f32(&self, input: &[f32], output: &mut [u8], _: bool) -> Option<()> {
        let mut indexed = vec![0.0; input.len() * self.base.num_components() as usize];

        for (input, output) in input
            .iter()
            .copied()
            .zip(indexed.chunks_exact_mut(self.base.num_components() as usize))
        {
            let idx = (input.clamp(0.0, self.hival as f32) + 0.5) as usize;
            output.copy_from_slice(&self.values[idx]);
        }

        self.base.convert_f32(&indexed, output, true)
    }
}
