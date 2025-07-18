use crate::function::{Clamper, Values};
use crate::object::Dict;
use crate::object::Number;
use crate::object::dict::keys::{C0, C1, N};
use smallvec::{SmallVec, smallvec};

/// A type 2 function (exponential function).
#[derive(Debug)]
pub(crate) struct Type2 {
    c0: Values,
    c1: Values,
    clamper: Clamper,
    n: f32,
}

impl Type2 {
    /// Create a new type 2 function.
    pub(crate) fn new(dict: &Dict) -> Option<Self> {
        let c0 = dict.get::<Values>(C0).unwrap_or(smallvec![0.0]);
        let c1 = dict.get::<Values>(C1).unwrap_or(smallvec![1.0]);
        let clamper = Clamper::new(dict)?;
        let n = dict.get::<Number>(N)?.as_f32();

        Some(Self { c0, c1, clamper, n })
    }

    /// Evaluate the function with the given input.
    pub(crate) fn eval(&self, input: f32) -> Values {
        let mut input = [input];
        self.clamper.clamp_input(&mut input);

        let mut out = self
            .c0
            .iter()
            .zip(self.c1.iter())
            .map(|(c0, c1)| *c0 + input[0].powf(self.n) * (*c1 - *c0))
            .collect::<SmallVec<_>>();

        self.clamper.clamp_output(&mut out);

        out
    }
}

#[cfg(test)]
mod tests {
    use crate::function::Function;

    use crate::object::Dict;
    use crate::object::Object;
    use crate::reader::Readable;
    use smallvec::smallvec;

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

        assert_eq!(func.eval(smallvec![0.0]).unwrap().as_ref(), &[0.0, 20.0]);
        assert_eq!(func.eval(smallvec![0.5]).unwrap().as_ref(), &[15.0, -15.0]);
        assert_eq!(func.eval(smallvec![1.0]).unwrap().as_ref(), &[30.0, -50.0]);
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

        assert_eq!(func.eval(smallvec![0.5]), Some(smallvec![7.5]));
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
            func.eval(smallvec![0.0]).as_ref(),
            func.eval(smallvec![0.2]).as_ref(),
        );
        assert_eq!(
            func.eval(smallvec![-10.]).as_ref(),
            func.eval(smallvec![0.2]).as_ref(),
        );
        assert_eq!(
            func.eval(smallvec![0.8]).as_ref(),
            func.eval(smallvec![0.8]).as_ref(),
        );
        assert_eq!(
            func.eval(smallvec![1.2]).as_ref(),
            func.eval(smallvec![1.0]).as_ref(),
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

        assert_eq!(func.eval(smallvec![0.0]), Some(smallvec![10.0]));
        assert_eq!(func.eval(smallvec![0.5]), Some(smallvec![15.0]));
        assert_eq!(func.eval(smallvec![1.0]), Some(smallvec![20.0]));
    }
}
