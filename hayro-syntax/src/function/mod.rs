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
use crate::object::Dict;
use crate::object::dict::keys::{DOMAIN, FUNCTION_TYPE, RANGE};
use crate::object::{Object, dict_or_stream};
use log::warn;
use smallvec::SmallVec;
use std::sync::Arc;

/// The input/output type of functions.
pub type Values = SmallVec<[f32; 4]>;
type TupleVec = SmallVec<[(f32, f32); 4]>;

#[derive(Debug)]
enum FunctionType {
    Type0(Type0),
    Type2(Type2),
    Type3(Type3),
    Type4(Type4),
}

/// A PDF function.
#[derive(Debug, Clone)]
pub struct Function(Arc<FunctionType>);

impl Function {
    /// Create a new function.
    pub fn new(obj: &Object) -> Option<Function> {
        let (dict, stream) = dict_or_stream(obj)?;

        let function_type = match dict.get::<u8>(FUNCTION_TYPE)? {
            0 => FunctionType::Type0(Type0::new(&stream?)?),
            2 => FunctionType::Type2(Type2::new(&dict)?),
            3 => FunctionType::Type3(Type3::new(&dict)?),
            4 => FunctionType::Type4(Type4::new(&stream?)?),
            _ => return None,
        };

        Some(Self(Arc::new(function_type)))
    }

    /// Evaluate the function with the given input.
    pub fn eval(&self, input: Values) -> Option<Values> {
        match self.0.as_ref() {
            FunctionType::Type0(t0) => t0.eval(input),
            FunctionType::Type2(t2) => Some(t2.eval(*input.first()?)),
            FunctionType::Type3(t3) => t3.eval(*input.first()?),
            FunctionType::Type4(t4) => Some(t4.eval(input)?),
        }
    }
}

#[derive(Debug, Clone)]
struct Clamper {
    domain: TupleVec,
    range: Option<TupleVec>,
}

impl Clamper {
    fn new(dict: &Dict) -> Option<Self> {
        let domain = dict.get::<TupleVec>(DOMAIN)?;
        let range = dict.get::<TupleVec>(RANGE);

        Some(Self { domain, range })
    }

    fn clamp_input(&self, input: &mut [f32]) {
        if input.len() != self.domain.len() {
            warn!("the domain of the function didn't match the input arguments");
        }

        for ((min, max), val) in self.domain.iter().zip(input.iter_mut()) {
            *val = val.clamp(*min, *max);
        }
    }

    fn clamp_output(&self, output: &mut [f32]) {
        if let Some(range) = &self.range {
            if range.len() != output.len() {
                warn!("the range of the function didn't match the output arguments");
            }

            for ((min, max), val) in range.iter().zip(output.iter_mut()) {
                *val = val.clamp(*min, *max);
            }
        }
    }
}

/// Linearly interpolate the value `x`, assuming that it lies within the range `x_min` and `x_max`,
/// to the range `y_min` and `y_max`.
pub fn interpolate(x: f32, x_min: f32, x_max: f32, y_min: f32, y_max: f32) -> f32 {
    y_min + (x - x_min) * (y_max - y_min) / (x_max - x_min)
}
