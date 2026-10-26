use super::{ColorSpace, ToRgb, U8Lookup};
use crate::cache::Cache;
use crate::function::{Function, MAX_COMPONENTS};
use hayro_syntax::object::{Array, Name, Object};

#[derive(Debug, Clone)]
pub(crate) struct Separation {
    alternate_space: ColorSpace,
    tint_transform: Function,
    is_none_separation: bool,
    lookup: U8Lookup<[u8; 3]>,
}

impl Separation {
    pub(super) fn new(array: &Array<'_>, cache: &Cache) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip `/Separation`
        let _ = iter.next::<Name<'_>>()?;
        let name = iter.next::<Name<'_>>()?;
        let alternate_space = ColorSpace::new(iter.next::<Object<'_>>()?, cache)?;
        let tint_transform = Function::new(&iter.next::<Object<'_>>()?)?;
        // Either I did something wrong, or no other viewers properly handles
        // `All`, so let's just ignore it as well.
        let is_none_separation = name.as_str() == "None";

        Some(Self {
            alternate_space,
            tint_transform,
            is_none_separation,
            lookup: U8Lookup::default(),
        })
    }

    fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let mut values = [0.0; MAX_COMPONENTS];
        let values = &mut values[..self.tint_transform.num_outputs()];
        let mut evaluated = Vec::with_capacity(input.len() * values.len());

        for n in input {
            if self
                .tint_transform
                .eval(&[*n as f32 * (1.0 / 255.0)], values)
                .is_some()
            {
                evaluated.extend(self.alternate_space.encode_values(values));
            } else {
                evaluated.extend(
                    self.alternate_space
                        .encode_values(&self.alternate_space.initial_color()),
                );
            }
        }

        self.alternate_space.convert(&evaluated, output)
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]; 256]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }
}

impl ToRgb for Separation {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let lookup = self.u8_lookup()?;
        for (input, output) in input.iter().zip(output.as_chunks_mut::<3>().0) {
            output.copy_from_slice(&lookup[*input as usize]);
        }

        Some(())
    }

    fn is_none(&self) -> bool {
        self.is_none_separation
    }
}
