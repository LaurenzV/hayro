use super::{ColorSpace, ToRgb};
use crate::cache::Cache;
use crate::function::Function;
use hayro_syntax::object::{Array, Name, Object};
use smallvec::ToSmallVec;

#[derive(Debug, Clone)]
pub(crate) struct DeviceN {
    alternate_space: ColorSpace,
    pub(super) num_components: u8,
    tint_transform: Function,
    is_none: bool,
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
        })
    }
}

impl ToRgb for DeviceN {
    fn convert_f32(&self, input: &[f32], output: &mut [u8], _: bool) -> Option<()> {
        let evaluated = input
            .chunks_exact(self.num_components as usize)
            .flat_map(|n| {
                self.tint_transform
                    .eval(n.to_smallvec())
                    .unwrap_or(self.alternate_space.initial_color())
            })
            .collect::<Vec<_>>();
        self.alternate_space.convert_f32(&evaluated, output, false)
    }

    fn is_none(&self) -> bool {
        self.is_none
    }
}
