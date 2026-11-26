use super::{ColorSpace, ColorSpaceType, ToRgb, U8Lookup, u8_to_f32};
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

    fn convert_inner(
        &self,
        input: &[u8],
        output: &mut [u8],
        remaining_cache: &mut usize,
    ) -> Option<()> {
        let n = usize::from(self.num_components);
        if !output.len().is_multiple_of(3) || input.len() / n != output.len() / 3 {
            return None;
        }
        let mut converter = BatchConverter::new(self);
        if input.len() < n {
            return converter.convert_alternate(output, remaining_cache);
        }
        for (src, dst) in input
            .chunks(BATCH_PIXELS * n)
            .zip(output.chunks_mut(BATCH_PIXELS * 3))
        {
            converter.convert(src, dst, remaining_cache)?;
        }
        Some(())
    }

    fn convert_lookup(
        &self,
        input: &[u8],
        output: &mut [u8],
        remaining_cache: &mut usize,
    ) -> Option<()> {
        let lookup = self
            .lookup
            .get_or_init(|tints, rgb| self.convert_inner(tints, rgb, remaining_cache))?;
        for (input, output) in input.iter().zip(output.as_chunks_mut::<3>().0) {
            output.copy_from_slice(&lookup[*input as usize]);
        }
        Some(())
    }
}

struct Stage<'a> {
    space: &'a DeviceN,
    samples: Vec<u8>,
    cached: Vec<u8>,
    offsets: FxHashMap<u64, usize>,
    previous: Option<(u64, usize)>,
}

struct BatchConverter<'a> {
    stages: Vec<Stage<'a>>,
    alternate: &'a ColorSpace,
}

impl<'a> BatchConverter<'a> {
    fn new(mut space: &'a DeviceN) -> Self {
        let mut stages = Vec::new();
        loop {
            stages.push(Stage {
                space,
                samples: Vec::new(),
                cached: Vec::new(),
                offsets: FxHashMap::default(),
                previous: None,
            });
            let mut alternate = &space.alternate_space;
            while let ColorSpaceType::Pattern(pattern) = alternate.0.as_ref() {
                alternate = pattern.color_space();
            }
            match alternate.0.as_ref() {
                ColorSpaceType::DeviceN(inner) if inner.num_components != 1 => space = inner,
                _ => return Self { stages, alternate },
            }
        }
    }

    fn convert(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        remaining_cache: &mut usize,
    ) -> Option<()> {
        self.stages[0].evaluate(input, remaining_cache);
        self.convert_alternate(output, remaining_cache)
    }

    fn convert_alternate(&mut self, output: &mut [u8], remaining_cache: &mut usize) -> Option<()> {
        let (first, rest) = self.stages.split_first_mut()?;
        let mut samples = &first.samples;
        for stage in rest {
            stage.evaluate(samples, remaining_cache);
            samples = &stage.samples;
        }
        if let ColorSpaceType::DeviceN(single) = self.alternate.0.as_ref() {
            single.convert_lookup(samples, output, remaining_cache)
        } else {
            self.alternate.convert(samples, output)
        }
    }
}

impl Stage<'_> {
    fn evaluate(&mut self, input: &[u8], remaining_cache: &mut usize) {
        let evaluated = &mut self.samples;
        let num_components = self.space.num_components as usize;
        let num_encoded = self.space.alternate_space.num_components() as usize;
        let mut components = [0.0; MAX_COMPONENTS];
        let mut values = [0.0; MAX_COMPONENTS];
        let num_outputs = self.space.tint_transform.num_outputs();
        let mut evaluate_pixel = |pixel: &[u8]| {
            let components = &mut components[..num_components];
            let values = &mut values[..num_outputs];
            for (component, value) in components.iter_mut().zip(pixel) {
                *component = u8_to_f32(*value);
            }

            if self.space.tint_transform.eval(components, values).is_some() {
                self.space.alternate_space.encode_values(values)
            } else {
                self.space
                    .alternate_space
                    .encode_values(&self.space.alternate_space.initial_color())
            }
        };
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

            let offset = match self.previous {
                Some((previous_key, offset)) if previous_key == key => Some(offset),
                _ => match self.offsets.get(&key) {
                    Some(&offset) => Some(offset),
                    None if *remaining_cache > 0 => {
                        let offset = self.cached.len();
                        self.cached.extend(evaluate_pixel(pixel));
                        self.offsets.insert(key, offset);
                        *remaining_cache -= 1;

                        Some(offset)
                    }
                    None => None,
                },
            };

            match offset {
                Some(offset) => {
                    evaluated.extend_from_slice(&self.cached[offset..offset + num_encoded]);
                    self.previous = Some((key, offset));
                }
                None => evaluated.extend(evaluate_pixel(pixel)),
            }
        }
    }
}

impl ToRgb for DeviceN {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let mut remaining_cache = MAX_CACHED_COLORS;
        if self.num_components == 1 {
            self.convert_lookup(input, output, &mut remaining_cache)
        } else {
            self.convert_inner(input, output, &mut remaining_cache)
        }
    }

    fn convert_in_place(&self, input: &mut [u8]) -> Option<()> {
        if self.num_components != 3 {
            return None;
        }

        if !input.len().is_multiple_of(3) {
            return None;
        }
        let mut converter = BatchConverter::new(self);
        let mut remaining_cache = MAX_CACHED_COLORS;
        if input.is_empty() {
            return converter.convert_alternate(input, &mut remaining_cache);
        }
        for chunk in input.chunks_mut(BATCH_PIXELS * 3) {
            converter.stages[0].evaluate(chunk, &mut remaining_cache);
            converter.convert_alternate(chunk, &mut remaining_cache)?;
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
5 0 obj << /FunctionType 4 /Domain [0 1 0 1 0 1] /Range [0 1 0 1 0 1] /Length 8 >>
stream
{ exch }
endstream
endobj
6 0 obj [/DeviceN [/A /B /C] [/DeviceN [/Tint] 4 0 R 7 0 R] 5 0 R] endobj
7 0 obj << /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [1 1 1] /N 1 >> endobj
8 0 obj [/DeviceN [/A /B /C] [/Pattern [/DeviceN [/X /Y /Z] /DeviceRGB 5 0 R]] 5 0 R] endobj
trailer << /Root 1 0 R >>
%%EOF"
                .to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn partial_pixel_initializes_alternate_lookup_with_shared_budget() {
        let pdf = test_spaces();
        let array = pdf.xref().get(ObjectIdentifier::new(6, 0)).unwrap();
        let space = DeviceN::new(&array, &Cache::new(), 0).unwrap();
        let mut remaining = 4;
        space
            .convert_inner(&[42, 7], &mut [], &mut remaining)
            .unwrap();
        assert_eq!(remaining, 0);
        let mut remaining = 4;
        space.convert_inner(&[], &mut [], &mut remaining).unwrap();
        assert_eq!(remaining, 4);
    }

    #[test]
    fn pattern_alternate_reuses_stages_and_shares_budget() {
        let pdf = test_spaces();
        let array = pdf.xref().get(ObjectIdentifier::new(8, 0)).unwrap();
        let space = DeviceN::new(&array, &Cache::new(), 0).unwrap();
        let mut converter = BatchConverter::new(&space);
        let mut remaining = MAX_CACHED_COLORS;
        let mut output = [0; 6];
        for _ in 0..2 {
            converter
                .convert(&[1, 2, 3, 4, 5, 6], &mut output, &mut remaining)
                .unwrap();
            assert_eq!(output, [1, 2, 3, 4, 5, 6]);
            assert_eq!(remaining, MAX_CACHED_COLORS - 4);
        }
    }

    #[test]
    fn multi_batch_tints_match_whole_image() {
        let pdf = test_spaces();
        let array = pdf.xref().get(ObjectIdentifier::new(4, 0)).unwrap();
        let space = DeviceN::new(&array, &Cache::new(), 0).unwrap();
        let mut input: Vec<_> = (0..(BATCH_PIXELS * 2 + 17) * 3)
            .map(|i| (i % 251) as u8)
            .collect();
        let mut whole = BatchConverter::new(&space);
        whole.stages[0].evaluate(&input, &mut 0);
        let mut expected = vec![0; input.len()];
        space
            .alternate_space
            .convert(&whole.stages[0].samples, &mut expected)
            .unwrap();
        assert_eq!(input, expected); // Both tint transforms exchange the last two components.
        let mut batches = BatchConverter::new(&space);
        let budget = 503;
        let mut remaining = budget;
        let mut batch_output = vec![0; BATCH_PIXELS * 3];
        batches
            .convert(
                &input[..batch_output.len()],
                &mut batch_output,
                &mut remaining,
            )
            .unwrap();
        let first_keys = batches.stages[1].offsets.clone();
        assert_eq!(first_keys.len(), 251);
        batches
            .convert(&[0; 6], &mut batch_output[..6], &mut remaining)
            .unwrap();
        let cache = &batches.stages[1].offsets;
        assert_eq!(cache.len() + batches.stages[0].offsets.len(), budget);
        assert_eq!(remaining, 0);
        assert_eq!(batch_output[..6], [0; 6]);
        assert_eq!(cache.len(), 251);
        assert!(
            first_keys
                .iter()
                .all(|(key, offset)| cache.get(key) == Some(offset))
        );
        batches
            .convert(
                &input[..batch_output.len()],
                &mut batch_output,
                &mut remaining,
            )
            .unwrap();
        assert_eq!(batch_output, expected[..batch_output.len()]);
        assert_eq!(
            space.convert(&[42, 7], &mut []),
            space.convert(&[], &mut [])
        );
        let mut output = vec![0; input.len()];
        space.convert(&input, &mut output).unwrap();
        assert_eq!(output, expected);
        space.convert_in_place(&mut input).unwrap();
        assert_eq!(input, expected);
    }
}
