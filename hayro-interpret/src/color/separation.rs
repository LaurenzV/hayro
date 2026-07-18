use super::{ColorSpace, ToRgb, U8Lookup, apply_u8_lookup};
use crate::cache::Cache;
use crate::function::Function;
use hayro_syntax::object::{Array, Name, Object};
use smallvec::smallvec;

#[derive(Debug, Clone)]
pub(crate) struct Separation {
    alternate_space: ColorSpace,
    tint_transform: Function,
    is_none_separation: bool,
    lookup: U8Lookup,
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
        let evaluated = input
            .iter()
            .flat_map(|n| {
                let values = self
                    .tint_transform
                    .eval(smallvec![*n as f32 / 255.0])
                    .unwrap_or(self.alternate_space.initial_color());
                self.alternate_space.encode_values(&values)
            })
            .collect::<Vec<u8>>();
        self.alternate_space.convert(&evaluated, output)
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }
}

impl ToRgb for Separation {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        apply_u8_lookup(input, output, self.u8_lookup()?);

        Some(())
    }

    fn is_none(&self) -> bool {
        self.is_none_separation
    }
}
