use crate::function::{Interval, MAX_COMPONENTS, read_intervals};
use hayro_syntax::bit_reader::BitReader;
use hayro_syntax::object::Array;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{BITS_PER_SAMPLE, DECODE, ENCODE, ORDER, SIZE};

#[derive(Debug)]
pub(crate) struct Type0 {
    order: u8,
    inputs: Vec<SampledInput>,
    decode: Vec<Interval>,
    sample_range: Interval,
    samples: Vec<f32>,
}

#[derive(Debug)]
struct SampledInput {
    domain: Interval,
    encode: Interval,
    size: u32,
    stride: usize,
}

impl SampledInput {
    fn sample_position(&self, x: f32) -> (usize, Option<f32>) {
        let last = self.size - 1;
        let position = self.domain.map(x, self.encode).max(0.0);
        let floor = position.floor();
        let index = (floor as u32).min(last);
        let fraction = position - floor;

        (
            index as usize,
            (index < last && fraction > 0.0).then_some(fraction),
        )
    }
}

impl Type0 {
    pub(crate) fn new(
        stream: &Stream<'_>,
        domain: &[Interval],
        range: &[Interval],
    ) -> Option<Self> {
        let dict = stream.dict();
        let bits_per_sample = dict.get::<u8>(BITS_PER_SAMPLE)?;

        if !matches!(bits_per_sample, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32) {
            return None;
        }

        let sizes = dict
            .get::<Array<'_>>(SIZE)?
            .iter::<u32>()
            .collect::<Vec<_>>();

        if sizes.len() != domain.len() || sizes.contains(&0) {
            return None;
        }

        let encode = read_intervals(dict, ENCODE).unwrap_or_else(|| {
            sizes
                .iter()
                .map(|size| Interval {
                    min: 0.0,
                    max: (size - 1) as f32,
                })
                .collect()
        });
        let decode = read_intervals(dict, DECODE).unwrap_or_else(|| range.to_vec());

        if encode.len() != sizes.len() || decode.len() != range.len() {
            return None;
        }

        let mut stride = range.len();
        let mut inputs = Vec::with_capacity(sizes.len());

        for ((domain, encode), size) in domain.iter().zip(encode).zip(sizes) {
            inputs.push(SampledInput {
                domain: *domain,
                encode,
                size,
                stride,
            });
            stride = stride.checked_mul(size as usize)?;
        }

        let decoded = stream.decoded_data().ok()?;

        if decoded.len() * 8 / usize::from(bits_per_sample) < stride {
            return None;
        }

        let mut reader = BitReader::new(&decoded);
        let samples = (0..stride)
            .map(|_| reader.read(bits_per_sample).map(|sample| sample as f32))
            .collect::<Option<Vec<_>>>()?;

        Some(Self {
            order: dict.get::<u8>(ORDER).unwrap_or(1),
            inputs,
            decode,
            sample_range: Interval {
                min: 0.0,
                max: ((1_u64 << bits_per_sample) - 1) as f32,
            },
            samples,
        })
    }

    pub(super) fn same_cell(&self, range: [f64; 2]) -> bool {
        let i = &self.inputs[0];
        let [a, b] = range.map(|x| {
            i.domain
                .map_f64(x, i.encode)
                .clamp(0., f64::from(i.size - 1))
        });
        i.domain.monotone_map(i.encode)
            && self
                .decode
                .iter()
                .all(|&d| self.sample_range.monotone_map(d))
            && (a.floor() == b.floor() || a.min(b).floor() + 1. >= a.max(b))
    }

    pub(super) fn eval_1d(&self, x: f64, output: &mut [f32]) {
        let input = &self.inputs[0];
        let last = input.size as usize - 1;
        let pos = input
            .domain
            .map_f64(x, input.encode)
            .clamp(0.0, last as f64);
        let index = pos as usize;
        let first = index * input.stride;
        let next = (index + 1).min(last) * input.stride;
        for (i, value) in output.iter_mut().enumerate() {
            let a = f64::from(self.samples[first + i]);
            let b = f64::from(self.samples[next + i]);
            *value = self
                .sample_range
                .map_f64(a + (b - a) * pos.fract(), self.decode[i]) as f32;
        }
    }

    pub(super) fn linear_pieces(
        &self,
        domain: [f64; 2],
        offsets: [f64; 2],
        budget: &mut usize,
        emit: &mut super::PieceEmitter<'_>,
    ) -> Option<()> {
        let [input] = self.inputs.as_slice() else {
            return None;
        };
        if !(self.order == 1 || self.order == 3 && input.size <= 3) {
            return None;
        }
        let [first, last] = domain.map(|x| input.domain.map_f64(x, input.encode));
        if !first.is_finite() || !last.is_finite() {
            return None;
        }
        let size = i64::from(input.size);
        let low = ((first.min(last).floor() + 1.0) as i64).clamp(0, size);
        let high = (first.max(last).ceil() as i64).clamp(low, size);
        *budget = budget.checked_sub((high - low) as usize + 1)?;
        let mut previous = [0.0; MAX_COMPONENTS];
        self.eval_1d(domain[0], &mut previous[..input.stride]);
        let mut start = offsets[0] as f32;
        for k in low..high {
            let index = if first < last {
                k
            } else {
                high - 1 - (k - low)
            };
            let end = (offsets[0]
                + (offsets[1] - offsets[0]) * ((index as f64 - first) / (last - first)))
                as f32;
            let mut color = [0.0; MAX_COMPONENTS];
            for (i, &decode) in self.decode.iter().enumerate() {
                color[i] = self
                    .sample_range
                    .map(self.samples[index as usize * input.stride + i], decode);
            }
            emit(
                [start, end],
                &previous[..input.stride],
                &color[..input.stride],
                budget,
            )?;
            previous = color;
            start = end;
        }
        let mut color = [0.0; MAX_COMPONENTS];
        self.eval_1d(domain[1], &mut color[..input.stride]);
        emit(
            [start, offsets[1] as f32],
            &previous[..input.stride],
            &color[..input.stride],
            budget,
        )?;
        Some(())
    }

    pub(crate) fn eval(&self, input: &[f32], output: &mut [f32]) {
        let mut offset = 0;
        let mut fractions = [(0, 0.0); MAX_COMPONENTS];
        let mut num_fractions = 0;

        for (x, input) in input.iter().zip(&self.inputs) {
            let (index, fraction) = input.sample_position(*x);
            offset += index * input.stride;

            if let Some(fraction) = fraction {
                fractions[num_fractions] = (input.stride, fraction);
                num_fractions += 1;
            }
        }

        let fractions = &fractions[..num_fractions];
        output.fill(0.0);

        for corner in 0..1_usize << fractions.len() {
            let mut weight = 1.0;
            let mut corner_offset = offset;

            for (i, (stride, fraction)) in fractions.iter().enumerate() {
                if corner >> (fractions.len() - 1 - i) & 1 == 1 {
                    weight *= fraction;
                    corner_offset += stride;
                } else {
                    weight *= 1.0 - fraction;
                }
            }

            let samples = &self.samples[corner_offset..corner_offset + output.len()];

            for (output, sample) in output.iter_mut().zip(samples) {
                *output += weight * *sample;
            }
        }

        for (output, decode) in output.iter_mut().zip(&self.decode) {
            *output = self.sample_range.map(*output, *decode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SampledInput;
    use crate::function::Interval;

    #[test]
    fn large_sample_table_skips_stop_derivation() {
        let unit = Interval { min: 0.0, max: 1.0 };
        let size = 1_048_577;
        let function = super::Type0 {
            order: 1,
            inputs: vec![SampledInput {
                domain: unit,
                encode: Interval {
                    min: 0.0,
                    max: (size - 1) as f32,
                },
                size,
                stride: 1,
            }],
            decode: vec![unit],
            sample_range: unit,
            samples: vec![0.0; size as usize],
        };
        let mut budget = super::super::MAX_LINEAR_PIECES;
        assert!(
            function
                .linear_pieces(
                    [0.0, 1.0],
                    [0.0, 1.0],
                    &mut budget,
                    &mut |_, _, _, _| panic!("unexpected native piece")
                )
                .is_none()
        );
        assert_eq!(budget, super::super::MAX_LINEAR_PIECES);
        let mut color = [1.0];
        function.eval(&[0.5], &mut color);
        assert_eq!(color, [0.0]);
    }

    #[test]
    fn sample_order_and_subset_encode() {
        use crate::function::Function;
        use hayro_syntax::{
            Pdf,
            object::{Object, ObjectIdentifier},
        };
        for (order, size, expected) in [
            ("", 4, 3),
            ("/Order 1", 4, 3),
            ("/Order 3", 4, 0),
            ("/Order 3", 3, 3),
        ] {
            let data = format!(
                "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\n3 0 obj << /FunctionType 0 /Domain [0 2] /Range [0 1] /Encode [3 0] /Size [{size}] /BitsPerSample 8 /Length 4 {order} >>\nstream\nABCD\nendstream\nendobj\ntrailer << /Root 1 0 R >>\n%%EOF"
            );
            let pdf = Pdf::new(data.into_bytes()).unwrap();
            let obj = pdf
                .xref()
                .get::<Object<'_>>(ObjectIdentifier::new(3, 0))
                .unwrap();
            let function = Function::new(&obj).unwrap();
            let mut pieces = 0;
            let result = function.linear_pieces(
                [1.5, 0.5],
                [0.0, 1.0],
                &mut 3,
                Some(0.01),
                &mut |offsets, a, b| {
                    for (offset, c) in offsets.into_iter().zip([a, b]) {
                        let expected = function.eval_one(&[1.5 - offset]).unwrap();
                        assert!((c[0] - expected[0]).abs() < 1e-6);
                    }
                    pieces += 1;
                    Some(())
                },
            );
            assert_eq!(result.is_some(), expected != 0);
            assert_eq!(pieces, expected);
        }
    }

    #[test]
    fn sample_index_stays_below_size_when_size_rounds() {
        let size = 16_777_220;
        let input = SampledInput {
            domain: Interval { min: 0.0, max: 1.0 },
            encode: Interval {
                min: 0.0,
                max: (size - 1) as f32,
            },
            size,
            stride: 1,
        };

        assert_eq!(input.sample_position(1.0), (16_777_219, None));
        assert_eq!(input.sample_position(2.0), (16_777_219, None));
    }
}
