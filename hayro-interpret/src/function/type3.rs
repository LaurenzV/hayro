use crate::function::{Function, Interval, interpolate, read_intervals};
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Object;
use hayro_syntax::object::dict::keys::{BOUNDS, ENCODE, FUNCTIONS};

/// A type 3 function (stitching function).
#[derive(Debug)]
pub(crate) struct Type3 {
    functions: Vec<Function>,
    bounds: Vec<f32>,
    encode: Vec<Interval>,
}

impl Type3 {
    /// Create a new type 3 function.
    pub(crate) fn new(dict: &Dict<'_>, domain: Interval) -> Option<Self> {
        let functions = dict
            .get::<Array<'_>>(FUNCTIONS)?
            .iter::<Object<'_>>()
            .map(|o| Function::new(&o))
            .collect::<Option<Vec<_>>>()?;

        let num_outputs = functions.first().map(Function::num_outputs)?;

        if functions
            .iter()
            .any(|f| f.num_inputs() != 1 || f.num_outputs() != num_outputs)
        {
            return None;
        }

        let inner_bounds = dict
            .get::<Array<'_>>(BOUNDS)
            .map(|a| a.iter::<f32>().collect::<Vec<_>>())
            .unwrap_or_default();
        let encode = read_intervals(dict, ENCODE)?;

        if inner_bounds.len() != functions.len() - 1 || encode.len() != functions.len() {
            return None;
        }

        let mut bounds = vec![domain.min - 0.0001];
        bounds.extend(inner_bounds);
        // Add a small delta so that the interval is considered to be closed on the right.
        bounds.push(domain.max + 0.0001);

        Some(Self {
            functions,
            bounds,
            encode,
        })
    }

    pub(crate) fn num_outputs(&self) -> usize {
        self.functions[0].num_outputs()
    }

    /// Evaluate the function with the given input.
    pub(crate) fn eval(&self, input: f32, output: &mut [f32]) -> Option<()> {
        let index = find_interval(&self.bounds, input)?;
        let interval = Interval {
            min: self.bounds[index],
            max: self.bounds[index + 1],
        };

        self.functions[index].eval(&[interval.map(input, self.encode[index])], output)
    }

    pub(crate) fn stitching_bounds(&self) -> Vec<f32> {
        let mut stitching_bounds = self.bounds[1..self.bounds.len() - 1].to_vec();

        for (index, function) in self.functions.iter().enumerate() {
            let (bounds_i_minus_1, bounds_i) = (self.bounds[index], self.bounds[index + 1]);
            let Interval {
                min: encode_min,
                max: encode_max,
            } = self.encode[index];

            if (encode_max - encode_min).abs() <= f32::EPSILON {
                continue;
            }

            for child_bound in function.stitching_bounds() {
                let bound = interpolate(
                    child_bound,
                    encode_min,
                    encode_max,
                    bounds_i_minus_1,
                    bounds_i,
                );
                let min_bound = bounds_i_minus_1.min(bounds_i);
                let max_bound = bounds_i_minus_1.max(bounds_i);
                if bound > min_bound && bound < max_bound {
                    stitching_bounds.push(bound);
                }
            }
        }

        stitching_bounds.sort_by(f32::total_cmp);
        stitching_bounds.dedup_by(|a, b| (*a - *b).abs() <= f32::EPSILON);

        stitching_bounds
    }
}

fn find_interval(bounds: &[f32], x: f32) -> Option<usize> {
    if x < *bounds.first()? || x >= *bounds.last()? {
        return None;
    }

    match bounds.binary_search_by(|val| {
        if *val <= x {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    }) {
        Ok(i) => Some(i - 1),
        Err(i) => Some(i - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro_syntax::object::FromBytes;

    #[test]
    fn simple() {
        let data = b"<<
  /FunctionType 3
  /Domain [-7 7]
  /Functions [
    << /FunctionType 2
       /Domain [0 1]
       /C0 [0.5 0.5 0.5]
       /C1 [0.5 0.5 0.5]
       /N 1
    >>
    << /FunctionType 2
       /Domain [0 1]
       /C0 [0.7 0.7 0.7]
       /C1 [0.7 0.7 0.7]
       /N 1
    >>
  ]
  /Bounds [0]
  /Encode [0 1 0 1]
>>";

        let dict = Object::from_bytes(data).unwrap();
        let function = Function::new(&dict).unwrap();

        assert_eq!(
            function.eval_one(&[-7.0]).unwrap().as_slice(),
            &[0.5, 0.5, 0.5]
        );
        assert_eq!(
            function.eval_one(&[-3.0]).unwrap().as_slice(),
            &[0.5, 0.5, 0.5]
        );
        assert_eq!(
            function.eval_one(&[-0.5]).unwrap().as_slice(),
            &[0.5, 0.5, 0.5]
        );
        assert_eq!(
            function.eval_one(&[0.0]).unwrap().as_slice(),
            &[0.7, 0.7, 0.7]
        );
        assert_eq!(
            function.eval_one(&[7.0]).unwrap().as_slice(),
            &[0.7, 0.7, 0.7]
        );
    }
}
