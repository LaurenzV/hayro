use super::{ColorComponent, ColorComponentSlice, ColorSpace, ToRgb, U8Lookup, apply_u8_lookup};
use crate::cache::Cache;
use crate::function::Function;
use hayro_syntax::object::{Array, Name, Object};

#[derive(Debug, Clone)]
pub(crate) struct DeviceN {
    alternate_space: ColorSpace,
    pub(super) num_components: u8,
    tint_transform: Function,
    is_none: bool,
    lookup: U8Lookup,
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

        if num_components == 0 {
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

    fn convert_inner<T: ColorComponent>(&self, input: &[T], output: &mut [u8]) -> Option<()> {
        let evaluated = input
            .chunks_exact(self.num_components as usize)
            .flat_map(|n| {
                let input = n.iter().map(|value| value.to_f32() / T::MAX_F32).collect();
                let values = self
                    .tint_transform
                    .eval(input)
                    .unwrap_or(self.alternate_space.initial_color());
                self.alternate_space.encode_values::<T>(&values)
            })
            .collect::<Vec<T>>();
        self.alternate_space.convert(&evaluated, output)
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }
}

impl ToRgb for DeviceN {
    fn convert<T: ColorComponent>(&self, input: &[T], output: &mut [u8]) -> Option<()> {
        if self.num_components == 1
            && let ColorComponentSlice::U8(input) = T::as_slice(input)
        {
            apply_u8_lookup(input, output, self.u8_lookup()?);

            Some(())
        } else {
            self.convert_inner(input, output)
        }
    }

    fn is_none(&self) -> bool {
        self.is_none
    }
}
