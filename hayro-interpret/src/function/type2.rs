use hayro_syntax::object::Dict;
use hayro_syntax::object::Number;
use hayro_syntax::object::dict::keys::{C0, C1, N};

/// A type 2 function (exponential function).
#[derive(Debug)]
pub(crate) struct Type2 {
    c0: Vec<f32>,
    delta: Vec<f32>,
    n: f32,
}

impl Type2 {
    /// Create a new type 2 function.
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

    pub(crate) fn num_outputs(&self) -> usize {
        self.c0.len()
    }

    /// Evaluate the function with the given input.
    pub(crate) fn eval(&self, input: f32, output: &mut [f32]) {
        let factor = if self.n == 1.0 {
            input
        } else {
            input.powf(self.n)
        };

        for ((output, c0), delta) in output.iter_mut().zip(&self.c0).zip(&self.delta) {
            *output = *c0 + factor * *delta;
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
