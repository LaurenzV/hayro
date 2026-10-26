use crate::function::{Interval, MAX_COMPONENTS, read_intervals};
use hayro_syntax::bit_reader::BitReader;
use hayro_syntax::object::Array;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{BITS_PER_SAMPLE, DECODE, ENCODE, SIZE};

/// A type 0 function (sampled function).
#[derive(Debug)]
pub(crate) struct Type0 {
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

impl Type0 {
    /// Create a new type 0 function.
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

        let decoded = stream.decoded().ok()?;

        if decoded.len() * 8 / usize::from(bits_per_sample) < stride {
            return None;
        }

        let mut reader = BitReader::new(&decoded);
        let samples = (0..stride)
            .map(|_| reader.read(bits_per_sample).map(|sample| sample as f32))
            .collect::<Option<Vec<_>>>()?;

        Some(Self {
            inputs,
            decode,
            sample_range: Interval {
                min: 0.0,
                max: ((1_u64 << bits_per_sample) - 1) as f32,
            },
            samples,
        })
    }

    /// Evaluate a type 0 function with the given input.
    pub(crate) fn eval(&self, input: &[f32], output: &mut [f32]) {
        let mut offset = 0;
        let mut fractions = [(0, 0.0); MAX_COMPONENTS];
        let mut num_fractions = 0;

        for (x, input) in input.iter().zip(&self.inputs) {
            let position = input
                .domain
                .map(*x, input.encode)
                .max(0.0)
                .min(input.size as f32 - 1.0);
            let index = position.floor();
            offset += index as usize * input.stride;

            if position != index {
                fractions[num_fractions] = (input.stride, position - index);
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
