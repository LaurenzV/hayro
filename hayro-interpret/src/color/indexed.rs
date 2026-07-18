use super::{ColorSpace, ToRgb, U8Lookup};
use crate::cache::Cache;
use hayro_syntax::object::{self, Array, Name, Object, Stream};
use std::borrow::Cow;

#[derive(Debug, Clone)]
pub(crate) struct Indexed {
    values: Vec<Vec<u8>>,
    hival: u8,
    base: Box<ColorSpace>,
    lookup: U8Lookup<[u8; 3]>,
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
                    temp.push(byte_iter.next()?);
                }

                vals.push(temp);
            }

            vals
        };

        Some(Self {
            values,
            hival,
            base: Box::new(base_color_space),
            lookup: U8Lookup::default(),
        })
    }

    pub(super) fn hival(&self) -> u8 {
        self.hival
    }

    fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let mut indexed = vec![0; input.len() * self.base.num_components() as usize];

        for (input, output) in input
            .iter()
            .zip(indexed.chunks_exact_mut(self.base.num_components() as usize))
        {
            let idx = (*input).min(self.hival) as usize;
            output.copy_from_slice(&self.values[idx]);
        }

        self.base.convert(&indexed, output)
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }
}

impl ToRgb for Indexed {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let lookup = self.u8_lookup()?;
        for (input, output) in input.iter().zip(output.chunks_exact_mut(3)) {
            output.copy_from_slice(&lookup[*input as usize]);
        }

        Some(())
    }
}
