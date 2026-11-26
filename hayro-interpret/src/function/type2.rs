use hayro_syntax::object::Dict;
use hayro_syntax::object::Number;
use hayro_syntax::object::dict::keys::{C0, C1, N};

#[derive(Debug)]
pub(crate) struct Type2 {
    c0: Vec<f32>,
    delta: Vec<f32>,
    n: f32,
}

impl Type2 {
    pub(crate) fn new(dict: &Dict<'_>) -> Option<Self> {
        let c0 = dict.get::<Vec<f32>>(C0).unwrap_or(vec![0.0]);
        let c1 = dict.get::<Vec<f32>>(C1).unwrap_or(vec![1.0]);
        if c0.len() != c1.len() {
            return None;
        }

        let delta = c0.iter().zip(&c1).map(|(c0, c1)| c1 - c0).collect();
        let n = dict.get::<Number>(N)?.as_f64() as f32;

        Some(Self { c0, delta, n })
    }

    pub(super) fn monotone(&self, [a, b]: [f64; 2]) -> bool {
        let lo = a.min(b);
        let hi = a.max(b);
        self.n.is_finite()
            && self.factor(a).is_finite()
            && self.factor(b).is_finite()
            && (lo >= 0. || hi <= 0. && self.n.fract() == 0.)
    }

    pub(super) fn square() -> Self {
        Self {
            c0: vec![0.],
            delta: vec![1.],
            n: 2.,
        }
    }

    pub(super) fn linear_pieces(
        &self,
        domain: [f64; 2],
        offsets: [f64; 2],
        budget: &mut usize,
        tolerance: Option<f32>,
        emit: &mut super::PieceEmitter<'_>,
    ) -> Option<()> {
        let [mut left, mut right] = [[0.0; super::MAX_COMPONENTS]; 2];
        let left = &mut left[..self.num_outputs()];
        let right = &mut right[..self.num_outputs()];
        let [first, last] = domain.map(|x| self.factor(x));
        self.apply(first, left);
        self.apply(last, right);
        if self.n == 1.0
            || self.n == 0.0
            || self.delta.iter().all(|&d| d == 0.)
                && first.is_finite()
                && last.is_finite()
                && self.monotone(domain)
        {
            *budget = budget.checked_sub(1)?;
            return emit(offsets.map(|t| t as f32), left, right, budget);
        }
        let min = domain[0].min(domain[1]);
        let max = domain[0].max(domain[1]);
        if min < 0.0 && max > 0.0 && self.n > 0.0 && self.n.fract() == 0.0 {
            let split =
                offsets[0] + (offsets[1] - offsets[0]) * (-domain[0] / (domain[1] - domain[0]));
            self.linear_pieces(
                [domain[0], 0.0],
                [offsets[0], split],
                budget,
                tolerance,
                emit,
            )?;
            return self.linear_pieces(
                [0.0, domain[1]],
                [split, offsets[1]],
                budget,
                tolerance,
                emit,
            );
        }
        let arc = (min >= 0.0 && self.n > 0.0)
            || (min > 0.0 && self.n < 0.0)
            || (max <= 0.0 && self.n.fract() == 0.0 && (self.n > 0.0 || max < 0.0));
        if !self.n.is_finite() || !arc {
            return None;
        }
        if tolerance.is_none() {
            *budget = budget.checked_sub(1)?;
            return emit(offsets.map(|t| t as f32), left, right, budget);
        }
        // On a convex or concave arc the maximum chord error is at most twice the midpoint error.
        let midpoint_tolerance = f64::from(tolerance?) * 0.5;
        let max_delta = self.delta.iter().copied().map(f32::abs).fold(0.0, f32::max);
        let mut pending = smallvec::SmallVec::<[_; 8]>::new();
        pending.push((domain, offsets, [first, last]));
        while let Some(([a, b], [start, end], [fa, fb])) = pending.pop() {
            let mid = a.midpoint(b);
            let fm = self.factor(mid);
            if (fm - fa.midpoint(fb)).abs() * f64::from(max_delta) > midpoint_tolerance {
                let split = start.midpoint(end);
                if pending.len() + 2 > *budget
                    || split as f32 == start as f32
                    || split as f32 == end as f32
                {
                    return None;
                }
                pending.push(([mid, b], [split, end], [fm, fb]));
                pending.push(([a, mid], [start, split], [fa, fm]));
            } else {
                *budget = budget.checked_sub(1)?;
                self.apply(fa, left);
                self.apply(fb, right);
                emit([start as f32, end as f32], left, right, budget)?;
            }
        }
        Some(())
    }

    pub(crate) fn num_outputs(&self) -> usize {
        self.c0.len()
    }

    pub(crate) fn eval(&self, input: f64, output: &mut [f32]) {
        self.apply(self.factor(input), output);
    }

    fn factor(&self, input: f64) -> f64 {
        if self.n == 1.0 {
            input
        } else {
            input.powf(f64::from(self.n))
        }
    }

    fn apply(&self, factor: f64, output: &mut [f32]) {
        for ((output, c0), delta) in output.iter_mut().zip(&self.c0).zip(&self.delta) {
            *output = (f64::from(*c0) + factor * f64::from(*delta)) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::function::Function;

    use hayro_syntax::object::Object;
    use hayro_syntax::object::{Dict, FromBytes};

    #[test]
    fn simple() {
        let func = Function::new(&Object::Dict(
            Dict::from_bytes(
                b"<<
              /FunctionType 2
              /Domain [ 0  1 ]
              /C0 [ 0 20  ]
              /C1 [ 30 -50 ]
              /N 1
            >>",
            )
            .unwrap(),
        ))
        .unwrap();

        assert_eq!(func.eval_one(&[0.0]).unwrap(), [0.0, 20.0]);
        assert_eq!(func.eval_one(&[0.5]).unwrap(), [15.0, -15.0]);
        assert_eq!(func.eval_one(&[1.0]).unwrap(), [30.0, -50.0]);
    }

    #[test]
    fn with_exponent() {
        let func = Function::new(&Object::Dict(
            Dict::from_bytes(
                b"<<
              /FunctionType 2
              /Domain [ 0  1 ]
              /C0 [ 0  ]
              /C1 [ 30 ]
              /N 2
            >>",
            )
            .unwrap(),
        ))
        .unwrap();

        assert_eq!(func.eval_one(&[0.5]), Some(vec![7.5]));
    }

    #[test]
    fn clamp_domain() {
        let func = Function::new(&Object::Dict(
            Dict::from_bytes(
                b"<<
              /FunctionType 2
              /Domain [ 0.2  0.8 ]
              /C0 [ 0  ]
              /C1 [ 30 ]
              /N 2
            >>",
            )
            .unwrap(),
        ))
        .unwrap();

        assert_eq!(
            func.eval_one(&[0.0]).as_ref(),
            func.eval_one(&[0.2]).as_ref(),
        );
        assert_eq!(
            func.eval_one(&[-10.]).as_ref(),
            func.eval_one(&[0.2]).as_ref(),
        );
        assert_eq!(
            func.eval_one(&[0.8]).as_ref(),
            func.eval_one(&[0.8]).as_ref(),
        );
        assert_eq!(
            func.eval_one(&[1.2]).as_ref(),
            func.eval_one(&[1.0]).as_ref(),
        );
    }

    #[test]
    fn clamp_range() {
        let func = Function::new(&Object::Dict(
            Dict::from_bytes(
                b"<<
              /FunctionType 2
              /Domain [ 0.0  1.0 ]
              /Range [10.0 20.0]
              /C0 [ 0  ]
              /C1 [ 30 ]
              /N 1
            >>",
            )
            .unwrap(),
        ))
        .unwrap();

        assert_eq!(func.eval_one(&[0.0]), Some(vec![10.0]));
        assert_eq!(func.eval_one(&[0.5]), Some(vec![15.0]));
        assert_eq!(func.eval_one(&[1.0]), Some(vec![20.0]));
    }
}
