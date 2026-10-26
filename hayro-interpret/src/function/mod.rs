//! PDF functions.
//!
//! PDF has the concept of functions, representing objects that take a certain number of values
//! as input, do some processing on them and then return some output.

mod type0;
mod type2;
mod type3;
mod type4;

use crate::function::type0::Type0;
use crate::function::type2::Type2;
use crate::function::type3::Type3;
use crate::function::type4::Type4;
use hayro_syntax::object::Dict;
use hayro_syntax::object::dict::keys::{DOMAIN, FUNCTION_TYPE, RANGE};
use hayro_syntax::object::{Object, dict_or_stream};
use std::sync::{Arc, OnceLock};

pub(crate) const MAX_COMPONENTS: usize = 32;

/// The maximum number of functions that may be nested in each other through the
/// `/Functions` of stitching functions. Real documents nest at most a few levels;
/// deeper chains would otherwise overflow the stack when building, evaluating or
/// dropping the function.
const MAX_FUNCTION_DEPTH: u8 = 32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Interval {
    pub(crate) min: f32,
    pub(crate) max: f32,
}

impl Interval {
    pub(crate) fn clamp(self, x: f32) -> f32 {
        x.min(self.max).max(self.min)
    }

    pub(crate) fn map(self, x: f32, to: Self) -> f32 {
        if self.min == self.max {
            return to.min;
        }

        interpolate(x, self.min, self.max, to.min, to.max)
    }
}

fn read_intervals(dict: &Dict<'_>, key: &[u8]) -> Option<Vec<Interval>> {
    let intervals = dict.get::<Vec<(f32, f32)>>(key)?;

    Some(
        intervals
            .into_iter()
            .map(|(min, max)| Interval { min, max })
            .collect(),
    )
}

#[derive(Debug)]
enum FunctionType {
    Type0(Type0),
    Type2(Type2),
    Type3(Type3),
    Type4(Type4),
}

#[derive(Debug)]
struct Repr {
    domain: Vec<Interval>,
    range: Option<Vec<Interval>>,
    num_outputs: usize,
    function_type: FunctionType,
}

/// A PDF function.
#[derive(Debug, Clone)]
pub struct Function(Arc<Repr>);

impl Function {
    /// Create a new function.
    pub fn new(obj: &Object<'_>) -> Option<Self> {
        Self::new_nested(obj, 0)
    }

    /// Create a new function that is nested `depth` levels deep in another one.
    fn new_nested(obj: &Object<'_>, depth: u8) -> Option<Self> {
        if depth >= MAX_FUNCTION_DEPTH {
            return None;
        }

        let (dict, stream) = dict_or_stream(obj)?;
        let function_type = dict.get::<u8>(FUNCTION_TYPE)?;
        let domain = read_intervals(dict, DOMAIN)?;
        let range = read_intervals(dict, RANGE);

        if domain.is_empty() || domain.len() > MAX_COMPONENTS {
            return None;
        }

        if matches!(function_type, 2 | 3) && domain.len() != 1 {
            return None;
        }

        let (function_type, num_outputs) = match function_type {
            0 => {
                let range = range.as_ref()?;
                let function = Type0::new(stream?, &domain, range)?;

                (FunctionType::Type0(function), range.len())
            }
            2 => {
                let function = Type2::new(dict)?;
                let num_outputs = function.num_outputs();

                (FunctionType::Type2(function), num_outputs)
            }
            3 => {
                let function = Type3::new(dict, domain[0], depth)?;
                let num_outputs = function.num_outputs();

                (FunctionType::Type3(function), num_outputs)
            }
            4 => (
                FunctionType::Type4(Type4::new(stream?)?),
                range.as_ref()?.len(),
            ),
            _ => return None,
        };

        if num_outputs == 0 || num_outputs > MAX_COMPONENTS {
            return None;
        }

        Some(Self(Arc::new(Repr {
            domain,
            range,
            num_outputs,
            function_type,
        })))
    }

    /// The number of input values of each sample.
    pub fn num_inputs(&self) -> usize {
        self.0.domain.len()
    }

    /// The number of output values of each sample.
    pub fn num_outputs(&self) -> usize {
        self.0.num_outputs
    }

    /// Evaluate the function for one or more samples.
    ///
    /// `input` needs to consist of samples with [`Self::num_inputs`] values each, and
    /// `output` needs to have space for the same number of samples with
    /// [`Self::num_outputs`] values each. Returns `None` if the lengths don't match or the
    /// function couldn't be evaluated.
    pub fn eval(&self, input: &[f32], output: &mut [f32]) -> Option<()> {
        let repr = &self.0;
        let (num_inputs, num_outputs) = (self.num_inputs(), self.num_outputs());

        if !input.len().is_multiple_of(num_inputs)
            || input.len() / num_inputs * num_outputs != output.len()
        {
            return None;
        }

        let mut clamped = [0.0; MAX_COMPONENTS];
        let clamped = &mut clamped[..num_inputs];

        for (input, output) in input
            .chunks_exact(num_inputs)
            .zip(output.chunks_exact_mut(num_outputs))
        {
            for ((clamped, value), domain) in clamped.iter_mut().zip(input).zip(&repr.domain) {
                *clamped = domain.clamp(*value);
            }

            match &repr.function_type {
                FunctionType::Type0(f) => f.eval(clamped, output),
                FunctionType::Type2(f) => f.eval(clamped[0], output),
                FunctionType::Type3(f) => f.eval(clamped[0], output)?,
                FunctionType::Type4(f) => f.eval(clamped, output)?,
            }

            if let Some(range) = &repr.range {
                for (value, range) in output.iter_mut().zip(range) {
                    *value = range.clamp(*value);
                }
            }
        }

        Some(())
    }

    pub(crate) fn stitching_bounds(&self) -> Vec<f32> {
        match &self.0.function_type {
            FunctionType::Type3(t3) => t3.stitching_bounds(),
            _ => vec![],
        }
    }
}

/// A transfer function.
#[derive(Clone, Debug)]
pub struct TransferFunction {
    function: Function,
    samples: Arc<OnceLock<[u8; 256]>>,
}

impl TransferFunction {
    pub(crate) fn new(function: Function) -> Self {
        Self {
            function,
            samples: Arc::new(OnceLock::new()),
        }
    }

    /// Apply the transfer function to a buffer of values.
    pub fn apply_to(&self, values: &mut [u8]) {
        self.apply_to_stride(values, 1);
    }

    pub(crate) fn apply_to_stride(&self, values: &mut [u8], stride: usize) {
        let samples = self.samples();

        for value in values.iter_mut().step_by(stride) {
            *value = samples[*value as usize];
        }
    }

    pub(crate) fn apply_f32(&self, value: f32) -> f32 {
        self.samples()[(value * 255.0 + 0.5) as u8 as usize] as f32 / 255.0
    }

    fn samples(&self) -> &[u8; 256] {
        self.samples.get_or_init(|| {
            let mut output = [0.0; MAX_COMPONENTS];
            let output = &mut output[..self.function.num_outputs()];

            std::array::from_fn(|sample| {
                match self.function.eval(&[sample as f32 * (1.0 / 255.0)], output) {
                    Some(()) => (output[0] * 255.0 + 0.5) as u8,
                    None => sample as u8,
                }
            })
        })
    }
}

/// Linearly interpolate the value `x`, assuming that it lies within the range `x_min` and `x_max`,
/// to the range `y_min` and `y_max`.
pub(crate) fn interpolate(x: f32, x_min: f32, x_max: f32, y_min: f32, y_max: f32) -> f32 {
    y_min + (x - x_min) * (y_max - y_min) / (x_max - x_min)
}

#[cfg(test)]
impl Function {
    pub(crate) fn eval_one(&self, input: &[f32]) -> Option<Vec<f32>> {
        let mut output = vec![0.0; self.num_outputs()];
        self.eval(input, &mut output)?;

        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::{Function, MAX_FUNCTION_DEPTH};
    use hayro_syntax::Pdf;
    use hayro_syntax::object::{Object, ObjectIdentifier};

    /// Build a document whose objects 10 to `10 + len - 1` are stitching functions that
    /// each name the next one as their only function, and whose last object is a type 2
    /// function.
    fn stitching_chain(len: usize) -> Pdf {
        let mut data = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n"
            .to_vec();

        for i in 0..len {
            let num = 10 + i;
            let body = if i + 1 == len {
                "<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>".to_string()
            } else {
                format!(
                    "<< /FunctionType 3 /Domain [0 1] /Functions [{} 0 R] /Bounds [] /Encode [0 1] >>",
                    num + 1
                )
            };
            data.extend(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
        }

        data.extend(b"trailer\n<< /Root 1 0 R >>\n%%EOF\n");

        Pdf::new(data).unwrap()
    }

    fn function_of_chain(len: usize) -> Option<Function> {
        let pdf = stitching_chain(len);
        let object = pdf
            .xref()
            .get::<Object<'_>>(ObjectIdentifier::new(10, 0))
            .unwrap();

        Function::new(&object)
    }

    #[test]
    fn nested_functions_up_to_bound() {
        let function = function_of_chain(MAX_FUNCTION_DEPTH as usize).unwrap();
        assert_eq!(function.eval_one(&[0.25]).unwrap().as_slice(), &[0.25]);

        assert!(function_of_chain(MAX_FUNCTION_DEPTH as usize + 1).is_none());
    }

    #[test]
    fn deep_function_chain_fails() {
        // Without a bound, building this chain recurses 20,000 times and overflows the stack.
        assert!(function_of_chain(20_000).is_none());
    }
}
