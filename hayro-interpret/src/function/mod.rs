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
                let function = Type3::new(dict, domain[0])?;
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
