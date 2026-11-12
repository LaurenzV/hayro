use super::{ColorSpace, ToLuma, ToRgb, U8Lookup};
use crate::cache::Cache;
use hayro_syntax::object::{self, Array, Name, Object, Stream};
use std::borrow::Cow;

#[derive(Debug, Clone)]
pub(crate) struct Indexed {
    values: Vec<u8>,
    hival: u8,
    base: Box<ColorSpace>,
    lookup: U8Lookup<[u8; 3]>,
    luma_lookup: U8Lookup<u8>,
}

impl Indexed {
    pub(super) fn new(array: &Array<'_>, cache: &Cache, depth: u8) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip name
        let _ = iter.next::<Name<'_>>()?;
        let base_color_space = ColorSpace::new_nested(iter.next::<Object<'_>>()?, cache, depth)?;
        let hival = iter.next::<u32>()?.min(u8::MAX as u32) as u8;

        let values = {
            let data = iter
                .next::<Stream<'_>>()
                .and_then(|s| s.decoded_data().ok())
                .or_else(|| {
                    iter.next::<object::String<'_>>()
                        .map(|s| Cow::Owned(s.to_vec()))
                })?;

            let num_components = base_color_space.num_components();

            let len = (hival as usize + 1) * num_components as usize;

            data.get(..len)?.to_vec()
        };

        Some(Self {
            values,
            hival,
            base: Box::new(base_color_space),
            lookup: U8Lookup::default(),
            luma_lookup: U8Lookup::default(),
        })
    }

    pub(super) fn hival(&self) -> u8 {
        self.hival
    }

    pub(super) fn base(&self) -> &ColorSpace {
        &self.base
    }

    pub(super) fn lookup(&self) -> &[u8] {
        &self.values
    }

    fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let mut indexed = vec![0; input.len() * self.base.num_components() as usize];

        let n = self.base.num_components() as usize;

        for (input, output) in input.iter().zip(indexed.chunks_exact_mut(n)) {
            let idx = (*input).min(self.hival) as usize;
            output.copy_from_slice(&self.values[idx * n..(idx + 1) * n]);
        }

        self.base.convert(&indexed, output)
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]; 256]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }

    fn u8_luma_lookup(&self) -> Option<&[u8; 256]> {
        self.luma_lookup.get_or_init_with(|_| {
            let rgb = self.u8_lookup()?;
            if rgb
                .iter()
                .any(|pixel| pixel[0] != pixel[1] || pixel[0] != pixel[2])
            {
                return None;
            }

            Some(Box::new(core::array::from_fn(|index| rgb[index][0])))
        })
    }
}

impl ToRgb for Indexed {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let lookup = self.u8_lookup()?;
        for (input, output) in input.iter().zip(output.as_chunks_mut::<3>().0) {
            output.copy_from_slice(&lookup[*input as usize]);
        }

        Some(())
    }
}

impl ToLuma for Indexed {
    fn to_luma(&self, input: &mut [u8]) -> Option<()> {
        let lookup = self.u8_luma_lookup()?;

        for value in input {
            *value = lookup[*value as usize];
        }

        Some(())
    }
}
