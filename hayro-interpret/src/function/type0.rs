use crate::function::{Clamper, TupleVec, Values, interpolate};
use hayro_syntax::bit_reader::BitReader;
use hayro_syntax::object::Array;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{BITS_PER_SAMPLE, DECODE, ENCODE, SIZE};
use smallvec::{SmallVec, smallvec};

/// A type 0 function (sampled function).
#[derive(Debug)]
pub(crate) struct Type0 {
    sizes: IntVec,
    strides: Vec<usize>,
    table: Vec<u32>,
    clamper: Clamper,
    range: TupleVec,
    bits_per_sample: u8,
    encode: TupleVec,
    decode: TupleVec,
}

impl Type0 {
    /// Create a new type 0 function.
    pub(crate) fn new(stream: &Stream<'_>) -> Option<Self> {
        let dict = stream.dict();
        let bits_per_sample = dict.get::<u8>(BITS_PER_SAMPLE)?;

        if !matches!(bits_per_sample, 1 | 2 | 4 | 8 | 16 | 24 | 32) {
            error!("invalid bits per sample: {bits_per_sample}");

            return None;
        }

        let clamper = Clamper::new(dict)?;
        let range = clamper.range.clone()?;

        if range.is_empty() {
            warn!("encountered Type0 function with invalid range length 0.");

            return None;
        }

        let sizes = dict
            .get::<Array<'_>>(SIZE)?
            .iter::<u32>()
            .collect::<IntVec>();

        let encode = dict
            .get::<TupleVec>(ENCODE)
            .unwrap_or(sizes.iter().map(|s| (0.0, (*s - 1) as f32)).collect());

        let decode = dict.get::<TupleVec>(DECODE).unwrap_or(range.clone());

        let mut data = {
            let decoded = stream.decoded().ok()?;
            let mut buf = vec![];
            let mut reader = BitReader::new(&decoded);

            while let Some(data) = reader.read(bits_per_sample) {
                buf.push(data);
            }

            buf
        };

        let mut stride = range.len();
        let mut strides = Vec::with_capacity(sizes.len());
        for size in &sizes {
            strides.push(stride);
            stride = stride.checked_mul(*size as usize)?;
        }
        let num_expected_entries = stride;

        if data.len() != num_expected_entries {
            warn!("Type0 function didn't have the expected number of sample entries.");
            data.truncate(num_expected_entries);
        }

        Some(Self {
            sizes,
            strides,
            clamper,
            range,
            bits_per_sample,
            table: data,
            encode,
            decode,
        })
    }

    /// Evaluate a type 0 function with the given input.
    pub(crate) fn eval(&self, mut input: Values) -> Option<Values> {
        if input.len() != self.sizes.len() {
            warn!("wrong number of arguments for sampled function");

            return None;
        }

        self.clamper.clamp_input(&mut input);

        let mut key = input;

        for (((x, domain), encode), size) in key
            .iter_mut()
            .zip(self.clamper.domain.iter())
            .zip(self.encode.iter())
            .zip(self.sizes.iter())
        {
            *x = interpolate(*x, domain.0, domain.1, encode.0, encode.1);
            *x = x.max(0.0).min(*size as f32 - 1.0);
        }

        let in_prev = key.iter().map(|v| v.floor() as u32).collect::<IntVec>();
        let in_next = key.iter().map(|v| v.ceil() as u32).collect::<IntVec>();

        let interpolator =
            Interpolator::new(&key, in_prev, in_next, &self.strides, self.range.len());

        let interpolated = interpolator.interpolate(&self.table)?;

        let mut out = interpolated
            .iter()
            .zip(self.decode.iter())
            .map(|(x, decode)| {
                interpolate(
                    *x,
                    0.0,
                    (2_u32.pow(self.bits_per_sample as u32) - 1) as f32,
                    decode.0,
                    decode.1,
                )
            })
            .collect::<SmallVec<_>>();

        self.clamper.clamp_output(&mut out);

        Some(out)
    }
}

type FloatVec = SmallVec<[f32; 4]>;
type IntVec = SmallVec<[u32; 4]>;

// See <https://github.com/apache/pdfbox/blob/bb778d4784f354c36ce032e91a0cee2169a4c598/pdfbox/src/main/java/org/apache/pdfbox/pdmodel/common/function/PDFunctionType0.java#L252>
struct Interpolator<'a> {
    input: &'a [f32],
    strides: &'a [usize],
    in_prev: IntVec,
    in_next: IntVec,
    out_len: usize,
}

impl<'a> Interpolator<'a> {
    fn new(
        input: &'a [f32],
        in_prev: IntVec,
        in_next: IntVec,
        strides: &'a [usize],
        out_len: usize,
    ) -> Self {
        Self {
            input,
            in_prev,
            in_next,
            strides,
            out_len,
        }
    }

    fn interpolate(&self, table: &[u32]) -> Option<FloatVec> {
        let mut out = smallvec![0.0; self.out_len];
        self.interpolate_inner(0, 0, 1.0, table, &mut out)?;
        Some(out)
    }

    fn interpolate_inner(
        &self,
        step: usize,
        offset: usize,
        weight: f32,
        table: &[u32],
        out: &mut [f32],
    ) -> Option<()> {
        if step == self.input.len() {
            let sample = table.get(offset..offset + self.out_len)?;
            for (out, sample) in out.iter_mut().zip(sample) {
                *out += weight * *sample as f32;
            }
            return Some(());
        }

        let prev = self.in_prev[step];
        let next = self.in_next[step];
        let stride = self.strides[step];

        if prev == next {
            self.interpolate_inner(
                step + 1,
                offset + prev as usize * stride,
                weight,
                table,
                out,
            )
        } else {
            let next_weight = self.input[step] - prev as f32;
            self.interpolate_inner(
                step + 1,
                offset + prev as usize * stride,
                weight * (1.0 - next_weight),
                table,
                out,
            )?;
            self.interpolate_inner(
                step + 1,
                offset + next as usize * stride,
                weight * next_weight,
                table,
                out,
            )
        }
    }
}
