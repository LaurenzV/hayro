mod type0;
mod type2;
mod type3;
mod type4;

use crate::CacheKey;
use crate::function::type0::Type0;
use crate::function::type2::Type2;
use crate::function::type3::Type3;
use crate::function::type4::Type4;
use hayro_syntax::object::Dict;
use hayro_syntax::object::dict::keys::{DOMAIN, FUNCTION_TYPE, RANGE};
use hayro_syntax::object::{Object, dict_or_stream};
use std::sync::{Arc, OnceLock};

pub(crate) const MAX_COMPONENTS: usize = 32;

pub(crate) const MAX_LINEAR_PIECES: usize = 1024;

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
        self.map_f64(f64::from(x), to) as f32
    }

    fn monotone_map(self, to: Self) -> bool {
        // The two rounded products in map_f64 must move in the same direction.
        self.min == self.max || to.min.min(to.max) <= 0. && to.min.max(to.max) >= 0.
    }

    fn map_f64(self, x: f64, to: Self) -> f64 {
        if self.min == self.max || to.min == to.max {
            return f64::from(to.min);
        }

        let [x_min, x_max, y_min, y_max] = [self.min, self.max, to.min, to.max].map(f64::from);
        let t = (x - x_min) / (x_max - x_min);
        (1.0 - t) * y_min + t * y_max
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
    eval_steps: usize,
    domain: Vec<Interval>,
    range: Option<Vec<Interval>>,
    num_outputs: usize,
    function_type: FunctionType,
}

/// A PDF function.
#[derive(Debug, Clone)]
pub struct Function(Arc<Repr>);

type PieceEmitter<'a> = dyn FnMut([f32; 2], &[f32], &[f32], &mut usize) -> Option<()> + 'a;

impl Function {
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Create a new function.
    pub fn new(obj: &Object<'_>) -> Option<Self> {
        Self::new_nested(obj, 0)
    }

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
            4 => {
                let outputs = range.as_ref()?.len();
                (
                    FunctionType::Type4(Type4::new(stream?, domain.len(), outputs)?),
                    outputs,
                )
            }
            _ => return None,
        };

        if num_outputs == 0 || num_outputs > MAX_COMPONENTS {
            return None;
        }

        let eval_steps = (domain.len() + num_outputs).saturating_add(match &function_type {
            // Scalar sampled evaluation reads two neighbors per output.
            FunctionType::Type0(_) => 3 * (num_outputs + 1),
            FunctionType::Type2(_) => num_outputs + 1,
            FunctionType::Type3(f) => f.steps(),
            FunctionType::Type4(f) => f.steps(),
        });
        Some(Self(Arc::new(Repr {
            eval_steps,
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

    /// Evaluate the function on consecutive samples of `input`, writing them to `output`.
    pub fn eval(&self, input: &[f32], output: &mut [f32]) -> Option<()> {
        let repr = &self.0;
        let (num_inputs, num_outputs) = (self.num_inputs(), self.num_outputs());

        if !input.len().is_multiple_of(num_inputs)
            || input.len() / num_inputs * num_outputs != output.len()
        {
            return None;
        }

        match &repr.function_type {
            FunctionType::Type0(f) => self.eval_samples(input, output, |input, output| {
                f.eval(input, output);
                Some(())
            }),
            FunctionType::Type2(f) => self.eval_samples(input, output, |input, output| {
                f.eval(f64::from(input[0]), output);
                Some(())
            }),
            FunctionType::Type3(f) => self.eval_samples(input, output, |input, output| {
                f.eval(f64::from(input[0]), output)
            }),
            FunctionType::Type4(f) => self.eval_samples(input, output, f.evaluator()),
        }
    }

    pub(crate) fn eval_1d_f64(&self, x: f64, output: &mut [f32]) -> Option<()> {
        let repr = &self.0;
        let [domain] = repr.domain.as_slice() else {
            return None;
        };
        if output.len() != repr.num_outputs {
            return None;
        }
        let x = x.min(f64::from(domain.max)).max(f64::from(domain.min));
        match &repr.function_type {
            FunctionType::Type0(f) => f.eval_1d(x, output),
            FunctionType::Type2(f) => f.eval(x, output),
            FunctionType::Type3(f) => f.eval(x, output)?,
            FunctionType::Type4(f) => f.evaluator()(&[x as f32], output)?,
        }
        self.clamp_output(output);
        Some(())
    }

    pub(crate) fn ops_per_eval(&self) -> usize {
        match &self.0.function_type {
            FunctionType::Type0(_) => 1_usize << self.num_inputs().min(20),
            FunctionType::Type4(f) => f.ops_per_eval(),
            FunctionType::Type3(f) => f.ops_per_eval(),
            _ => 1,
        }
    }

    pub(crate) fn monotone(&self, range: [f64; 2]) -> Option<bool> {
        let [domain] = self.0.domain.as_slice() else {
            return None;
        };
        let range = range.map(|x| x.min(f64::from(domain.max)).max(f64::from(domain.min)));
        match &self.0.function_type {
            FunctionType::Type0(f) => Some(f.same_cell(range)),
            FunctionType::Type2(f) => Some(f.monotone(range)),
            FunctionType::Type3(f) => f.monotone(range),
            FunctionType::Type4(f) => f.monotone(range),
        }
    }

    fn clamp_output(&self, output: &mut [f32]) {
        if let Some(range) = &self.0.range {
            for (value, range) in output.iter_mut().zip(range) {
                *value = range.clamp(*value);
            }
        }
    }

    fn eval_samples(
        &self,
        input: &[f32],
        output: &mut [f32],
        mut eval: impl FnMut(&[f32], &mut [f32]) -> Option<()>,
    ) -> Option<()> {
        let repr = &self.0;
        let mut clamped = [0.0; MAX_COMPONENTS];
        let clamped = &mut clamped[..self.num_inputs()];

        for (input, output) in input
            .chunks_exact(self.num_inputs())
            .zip(output.chunks_exact_mut(self.num_outputs()))
        {
            for ((clamped, value), domain) in clamped.iter_mut().zip(input).zip(&repr.domain) {
                *clamped = domain.clamp(*value);
            }

            eval(clamped, output)?;

            self.clamp_output(output);
        }

        Some(())
    }

    pub(crate) fn linear_pieces(
        &self,
        domain: [f64; 2],
        offsets: [f64; 2],
        budget: &mut usize,
        tolerance: Option<f32>,
        emit: &mut impl FnMut([f32; 2], &[f32], &[f32]) -> Option<()>,
    ) -> Option<()> {
        self.ranged_pieces(domain, offsets, budget, tolerance, &mut |ts, a, b, _| {
            a.iter()
                .chain(b)
                .all(|v| (0.0..=1.0).contains(v))
                .then_some(())?;
            emit(ts, a, b)
        })
    }

    fn eval_piece(&self, x: f64, values: &mut [f32], budget: &mut usize) -> Option<()> {
        *budget = budget.checked_sub(self.0.eval_steps)?;
        self.eval_1d_f64(x, values)
    }

    fn ranged_pieces(
        &self,
        domain: [f64; 2],
        offsets: [f64; 2],
        budget: &mut usize,
        tolerance: Option<f32>,
        emit: &mut PieceEmitter<'_>,
    ) -> Option<()> {
        let repr = &self.0;
        let [bounds] = repr.domain.as_slice() else {
            return None;
        };
        if *budget == 0
            || !domain.iter().all(|x| x.is_finite())
            || bounds.min > bounds.max
            || !bounds.min.is_finite()
            || !bounds.max.is_finite()
        {
            return None;
        }
        let clipped = domain.map(|x| x.clamp(f64::from(bounds.min), f64::from(bounds.max)));
        let constant = |x, ts: [f64; 2], budget: &mut usize, emit: &mut PieceEmitter<'_>| {
            let mut values = [0.; MAX_COMPONENTS];
            let values = &mut values[..self.num_outputs()];
            self.eval_piece(x, values, budget)?;
            emit(ts.map(|t| t as f32), values, values, budget)
        };
        if clipped[0] == clipped[1] {
            if clipped[0].to_bits() != clipped[1].to_bits() {
                return None;
            }
            return constant(clipped[0], offsets, budget, emit);
        }
        if clipped != domain {
            let offset = |x| {
                offsets[0] + (x - domain[0]) / (domain[1] - domain[0]) * (offsets[1] - offsets[0])
            };
            let ts = [
                if clipped[0] == domain[0] {
                    offsets[0]
                } else {
                    offset(clipped[0])
                },
                if clipped[1] == domain[1] {
                    offsets[1]
                } else {
                    offset(clipped[1])
                },
            ];
            if clipped[0] != domain[0] {
                constant(clipped[0], [offsets[0], ts[0]], budget, emit)?;
            }
            self.ranged_pieces(clipped, ts, budget, tolerance, emit)?;
            if clipped[1] != domain[1] {
                constant(clipped[1], [ts[1], offsets[1]], budget, emit)?;
            }
            return Some(());
        }
        let mut clamp = |ts, a: &[f32], b: &[f32], remaining: &mut usize| {
            if tolerance.is_none() && self.0.range.is_some() {
                let mut left = smallvec::SmallVec::<[f32; 4]>::from_slice(a);
                let mut right = smallvec::SmallVec::<[f32; 4]>::from_slice(b);
                self.clamp_output(&mut left);
                self.clamp_output(&mut right);
                return emit(ts, &left, &right, remaining);
            }
            self.emit_clamped(ts, a, b, remaining, emit)
        };
        match &repr.function_type {
            FunctionType::Type0(f) => f.linear_pieces(domain, offsets, budget, &mut clamp),
            FunctionType::Type2(f) => {
                f.linear_pieces(domain, offsets, budget, tolerance, &mut clamp)
            }
            FunctionType::Type4(f)
                if f.is_square()
                    && self.num_outputs() == 1
                    && domain.iter().all(|x| x.abs() <= 1.) =>
            {
                Type2::square().linear_pieces(
                    domain,
                    offsets,
                    budget,
                    Some(tolerance? - f32::EPSILON),
                    &mut clamp,
                )
            }
            FunctionType::Type3(f) => {
                f.linear_pieces(domain, offsets, budget, tolerance, &mut clamp)
            }
            _ => None,
        }
    }

    fn emit_clamped(
        &self,
        ts: [f32; 2],
        a: &[f32],
        b: &[f32],
        remaining: &mut usize,
        emit: &mut PieceEmitter<'_>,
    ) -> Option<()> {
        let Some(ranges) = &self.0.range else {
            return emit(ts, a, b, remaining);
        };
        if a.iter()
            .zip(b)
            .zip(ranges)
            .all(|((&a, &b), r)| r.clamp(a) == a && r.clamp(b) == b)
        {
            return emit(ts, a, b, remaining);
        }
        if !a.iter().chain(b).all(|v| v.is_finite()) {
            return None;
        }
        let mut cuts = smallvec::SmallVec::<[f64; 4]>::new();
        for ((&a, &b), r) in a.iter().zip(b).zip(ranges) {
            let mut previous = None;
            for bound in [r.min, r.max] {
                if a.min(b) < bound && bound < a.max(b) {
                    let u = (f64::from(bound) - f64::from(a)) / (f64::from(b) - f64::from(a));
                    if r.min != r.max && previous == Some(u) {
                        return None;
                    }
                    cuts.push(u);
                    previous = Some(u);
                }
            }
        }
        cuts.sort_unstable_by(f64::total_cmp);
        cuts.dedup();
        *remaining = remaining.checked_sub(cuts.len())?;
        let mut left = [0.0; MAX_COMPONENTS];
        let left = &mut left[..a.len()];
        left.copy_from_slice(a);
        self.clamp_output(left);
        let mut start = ts[0];
        for u in cuts.into_iter().chain([1.0]) {
            let mut right = [0.0; MAX_COMPONENTS];
            let right = &mut right[..a.len()];
            for ((v, &a), &b) in right.iter_mut().zip(a).zip(b) {
                *v = ((1.0 - u) * f64::from(a) + u * f64::from(b)) as f32;
            }
            self.clamp_output(right);
            let end = ((1.0 - u) * f64::from(ts[0]) + u * f64::from(ts[1])) as f32;
            emit([start, end], left, right, remaining)?;
            left.copy_from_slice(right);
            start = end;
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
pub struct TransferFunction(Arc<TransferFunctionRepr>);

#[derive(Debug)]
struct TransferFunctionRepr {
    cache_key: u128,
    function: Function,
    samples: OnceLock<[u8; 256]>,
}

impl CacheKey for TransferFunction {
    fn cache_key(&self) -> u128 {
        self.0.cache_key
    }
}

impl TransferFunction {
    pub(crate) fn new(obj: &Object<'_>) -> Option<Self> {
        Some(Self(Arc::new(TransferFunctionRepr {
            function: Function::new(obj)?,
            cache_key: obj.cache_key(),
            samples: OnceLock::new(),
        })))
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
        self.0.samples.get_or_init(|| {
            let function = &self.0.function;
            let mut output = [0.0; MAX_COMPONENTS];
            let output = &mut output[..function.num_outputs()];

            std::array::from_fn(|sample| {
                match function.eval(&[sample as f32 * (1.0 / 255.0)], output) {
                    Some(()) => (output[0] * 255.0 + 0.5) as u8,
                    None => sample as u8,
                }
            })
        })
    }
}

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
        assert!(function_of_chain(20_000).is_none());
    }
}
