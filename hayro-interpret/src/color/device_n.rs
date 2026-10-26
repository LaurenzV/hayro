use super::{ColorSpace, ToRgb, U8Lookup};
use crate::cache::Cache;
use crate::function::{Function, MAX_COMPONENTS};
use hayro_syntax::object::{Array, Name, Object};

#[derive(Debug, Clone)]
pub(crate) struct DeviceN {
    alternate_space: ColorSpace,
    pub(super) num_components: u8,
    tint_transform: Function,
    is_none: bool,
    lookup: U8Lookup<[u8; 3]>,
}

impl DeviceN {
    pub(super) fn new(array: &Array<'_>, cache: &Cache) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip `/DeviceN`
        let _ = iter.next::<Name<'_>>()?;
        // Skip `Name`.
        let names = iter
            .next::<Array<'_>>()?
            .iter::<Name<'_>>()
            .collect::<Vec<_>>();
        let num_components = u8::try_from(names.len()).ok()?;
        let all_none = names.iter().all(|n| n.as_str() == "None");
        let alternate_space = ColorSpace::new(iter.next::<Object<'_>>()?, cache)?;
        let tint_transform = Function::new(&iter.next::<Object<'_>>()?)?;

        if num_components == 0 || num_components as usize > MAX_COMPONENTS {
            return None;
        }

        Some(Self {
            alternate_space,
            num_components,
            tint_transform,
            is_none: all_none,
            lookup: U8Lookup::default(),
        })
    }

    fn evaluate(&self, input: &[u8]) -> Vec<u8> {
        let num_components = self.num_components as usize;
        let mut components = [0.0; MAX_COMPONENTS];
        let components = &mut components[..num_components];
        let mut values = [0.0; MAX_COMPONENTS];
        let values = &mut values[..self.tint_transform.num_outputs()];
        let mut evaluated = Vec::with_capacity(
            input.len() / num_components * self.alternate_space.num_components() as usize,
        );

        for pixel in input.chunks_exact(num_components) {
            for (component, value) in components.iter_mut().zip(pixel) {
                *component = *value as f32 * (1.0 / 255.0);
            }

            if self.tint_transform.eval(components, values).is_some() {
                evaluated.extend(self.alternate_space.encode_values(values));
            } else {
                evaluated.extend(
                    self.alternate_space
                        .encode_values(&self.alternate_space.initial_color()),
                );
            }
        }

        evaluated
    }

    fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let evaluated = self.evaluate(input);
        self.alternate_space.convert(&evaluated, output)
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]; 256]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }
}

impl ToRgb for DeviceN {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        if self.num_components == 1 {
            let lookup = self.u8_lookup()?;
            for (input, output) in input.iter().zip(output.as_chunks_mut::<3>().0) {
                output.copy_from_slice(&lookup[*input as usize]);
            }

            Some(())
        } else {
            self.convert_inner(input, output)
        }
    }

    fn convert_in_place(&self, input: &mut [u8]) -> Option<()> {
        if self.num_components != 3 {
            return None;
        }

        let evaluated = self.evaluate(input);
        self.alternate_space.convert(&evaluated, input)
    }

    fn is_none(&self) -> bool {
        self.is_none
    }
}
