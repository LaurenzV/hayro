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

impl SampledInput {
    /// Return the index of the sample at or below the encoded position of `x`,
    /// and the fraction towards the next sample if there is one.
    ///
    /// The index is clamped as an integer, because `size - 1` may not be
    /// representable as an `f32`: for `size = 16_777_220`, it rounds up to
    /// `size`.
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

        let decoded = stream.decoded_data().ok()?;

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

    fn input(size: u32) -> SampledInput {
        SampledInput {
            domain: Interval { min: 0.0, max: 1.0 },
            encode: Interval {
                min: 0.0,
                max: (size - 1) as f32,
            },
            size,
            stride: 1,
        }
    }

    #[test]
    fn sample_index_stays_below_size_when_size_rounds() {
        // `16_777_219_f32` rounds to `16_777_220.0`.
        let input = input(16_777_220);

        assert_eq!(input.sample_position(1.0), (16_777_219, None));
        assert_eq!(input.sample_position(2.0), (16_777_219, None));
    }

    #[test]
    fn sample_position_interpolates_between_samples() {
        let input = input(5);

        assert_eq!(input.sample_position(0.0), (0, None));
        assert_eq!(input.sample_position(0.625), (2, Some(0.5)));
        assert_eq!(input.sample_position(1.0), (4, None));
        assert_eq!(input.sample_position(-1.0), (0, None));
        assert_eq!(input.sample_position(f32::NAN), (0, None));
    }
}
