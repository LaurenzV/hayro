use super::{ColorSpace, ToRgb, U8Lookup};
use crate::cache::Cache;
use crate::function::{Function, MAX_COMPONENTS};
use hayro_syntax::object::{Array, Name, Object};
use rustc_hash::FxHashMap;

const BATCH_PIXELS: usize = 4096;
const MAX_CACHED_COLORS: usize = 1 << 18;

#[derive(Debug, Clone)]
pub(crate) struct DeviceN {
    alternate_space: ColorSpace,
    pub(super) num_components: u8,
    tint_transform: Function,
    is_none: bool,
    lookup: U8Lookup<[u8; 3]>,
}

impl DeviceN {
    pub(super) fn new(array: &Array<'_>, cache: &Cache, depth: u8) -> Option<Self> {
        let mut iter = array.flex_iter();
        let _ = iter.next::<Name<'_>>()?;
        let names = iter
            .next::<Array<'_>>()?
            .iter::<Name<'_>>()
            .collect::<Vec<_>>();
        let num_components = u8::try_from(names.len()).ok()?;
        let all_none = names.iter().all(|n| n.as_str() == "None");
        let alternate_space = ColorSpace::new_nested(iter.next::<Object<'_>>()?, cache, depth)?;
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

    fn evaluator(&self) -> impl FnMut(&[u8], &mut Vec<u8>) + '_ {
        let num_components = self.num_components as usize;
        let num_encoded = self.alternate_space.num_components() as usize;
        let mut components = [0.0; MAX_COMPONENTS];
        let mut values = [0.0; MAX_COMPONENTS];
        let num_outputs = self.tint_transform.num_outputs();
        let mut evaluate_pixel = move |pixel: &[u8]| {
            let components = &mut components[..num_components];
            let values = &mut values[..num_outputs];
            for (component, value) in components.iter_mut().zip(pixel) {
                *component = *value as f32 * (1.0 / 255.0);
            }

            if self.tint_transform.eval(components, values).is_some() {
                self.alternate_space.encode_values(values)
            } else {
                self.alternate_space
                    .encode_values(&self.alternate_space.initial_color())
            }
        };
        let mut cached = Vec::new();
        let mut offsets = FxHashMap::default();
        let mut previous = None;

        move |input, evaluated| {
            evaluated.clear();
            evaluated.reserve_exact(input.len() / num_components * num_encoded);
            if num_components == 1 || num_components > 8 || input.len() == num_components {
                for pixel in input.chunks_exact(num_components) {
                    evaluated.extend(evaluate_pixel(pixel));
                }

                return;
            }

            for pixel in input.chunks_exact(num_components) {
                let key = pixel
                    .iter()
                    .fold(0_u64, |key, value| (key << 8) | u64::from(*value));

                let offset = match previous {
                    Some((previous_key, offset)) if previous_key == key => Some(offset),
                    _ => match offsets.get(&key) {
                        Some(&offset) => Some(offset),
                        None if offsets.len() < MAX_CACHED_COLORS => {
                            let offset = cached.len();
                            cached.extend(evaluate_pixel(pixel));
                            offsets.insert(key, offset);

                            Some(offset)
                        }
                        None => None,
                    },
                };

                match offset {
                    Some(offset) => {
                        evaluated.extend_from_slice(&cached[offset..offset + num_encoded]);
                        previous = Some((key, offset));
                    }
                    None => evaluated.extend(evaluate_pixel(pixel)),
                }
            }
        }
    }

    fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let n = usize::from(self.num_components);
        if !output.len().is_multiple_of(3) || input.len() / n != output.len() / 3 {
            return None;
        }
        let mut evaluate = self.evaluator();
        let mut buf = Vec::new();
        if input.len() < n {
            return self.alternate_space.convert(&[], output);
        }
        for (src, dst) in input
            .chunks(BATCH_PIXELS * n)
            .zip(output.chunks_mut(BATCH_PIXELS * 3))
        {
            evaluate(src, &mut buf);
            self.alternate_space.convert(&buf, dst)?;
        }
        Some(())
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

        let mut evaluate = self.evaluator();
        if !input.len().is_multiple_of(3) {
            return None;
        }
        let mut buf = Vec::new();
        if input.is_empty() {
            return self.alternate_space.convert(&buf, input);
        }
        for chunk in input.chunks_mut(BATCH_PIXELS * 3) {
            evaluate(chunk, &mut buf);
            self.alternate_space.convert(&buf, chunk)?;
        }
        Some(())
    }

    fn is_none(&self) -> bool {
        self.is_none
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro_syntax::{Pdf, object::ObjectIdentifier};

    fn test_spaces() -> Pdf {
        Pdf::new(
            b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj
4 0 obj [/DeviceN [/A /B /C] [/DeviceN [/X /Y /Z] /DeviceRGB 5 0 R] 5 0 R] endobj
5 0 obj << /FunctionType 4 /Domain [0 1 0 1 0 1] /Range [0 1 0 1 0 1] /Length 12 >>
stream
{ 3 1 roll }
endstream
endobj
6 0 obj [/DeviceN [/A /B /C] [/DeviceN [/Tint] /DeviceRGB 7 0 R] 5 0 R] endobj
7 0 obj << /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [1 1 1] /N 1 >> endobj
trailer << /Root 1 0 R >>
%%EOF"
                .to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn partial_pixel_initializes_the_alternate_lookup() {
        let pdf = test_spaces();
        let array = pdf.xref().get(ObjectIdentifier::new(6, 0)).unwrap();
        let space = DeviceN::new(&array, &Cache::new(), 0).unwrap();
        space.convert_inner(&[42, 7], &mut []).unwrap();
        let crate::color::ColorSpaceType::DeviceN(inner) = space.alternate_space.0.as_ref() else {
            panic!("expected DeviceN alternate")
        };
        assert!(inner.lookup.0.get().is_some());
    }

    #[test]
    fn nested_tints_match_across_batches() {
        let pdf = test_spaces();
        let array = pdf.xref().get(ObjectIdentifier::new(4, 0)).unwrap();
        let space = DeviceN::new(&array, &Cache::new(), 0).unwrap();
        for pixels in [0, BATCH_PIXELS, BATCH_PIXELS + 1] {
            let mut input: Vec<_> = (0..pixels * 3).map(|i| (i % 251) as u8).collect();
            let mut expected = input.clone();
            for pixel in expected.as_chunks_mut::<3>().0 {
                pixel.rotate_left(1);
            }
            let mut in_place = input.clone();
            space.convert_in_place(&mut in_place).unwrap();
            assert_eq!(in_place, expected);

            let mut output = vec![0; expected.len()];
            for trailing in [None, Some(42), Some(7)] {
                input.extend(trailing);
                space.convert_inner(&input, &mut output).unwrap();
                assert_eq!(output, expected);
            }
            assert!(space.convert_in_place(&mut input).is_none());
        }
    }
}
