use crate::function::{Function, Interval, read_intervals};
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Object;
use hayro_syntax::object::dict::keys::{BOUNDS, ENCODE, FUNCTIONS};

#[derive(Debug)]
pub(crate) struct Type3 {
    ordered_bounds: bool,
    functions: Vec<Function>,
    bounds: Vec<f32>,
    encode: Vec<Interval>,
}

impl Type3 {
    pub(super) fn steps(&self) -> usize {
        self.functions
            .iter()
            .map(|f| f.0.eval_steps)
            .max()
            .unwrap_or_default()
            .saturating_add(2 + self.bounds.len().ilog2() as usize)
    }

    pub(crate) fn new(dict: &Dict<'_>, domain: Interval, depth: u8) -> Option<Self> {
        let functions = dict
            .get::<Array<'_>>(FUNCTIONS)?
            .iter::<Object<'_>>()
            .map(|o| Function::new_nested(&o, depth + 1))
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

        let mut bounds = vec![domain.min];
        bounds.extend(inner_bounds);
        bounds.push(domain.max);

        Some(Self {
            ordered_bounds: bounds.windows(2).all(|p| p[0] <= p[1]),
            functions,
            bounds,
            encode,
        })
    }

    pub(crate) fn num_outputs(&self) -> usize {
        self.functions[0].num_outputs()
    }

    pub(crate) fn eval(&self, input: f64, output: &mut [f32]) -> Option<()> {
        let index = self.find_interval(input);
        let interval = Interval {
            min: self.bounds[index],
            max: self.bounds[index + 1],
        };

        self.functions[index].eval_1d_f64(interval.map_f64(input, self.encode[index]), output)
    }

    pub(super) fn ops_per_eval(&self) -> usize {
        self.functions
            .iter()
            .map(Function::ops_per_eval)
            .max()
            .unwrap_or(1)
    }

    pub(super) fn monotone(&self, range: [f64; 2]) -> Option<bool> {
        let [i, j] = range.map(|x| self.find_interval(x));
        if i != j {
            return Some(false);
        }
        let domain = Interval {
            min: self.bounds[i],
            max: self.bounds[i + 1],
        };
        if !domain.monotone_map(self.encode[i]) {
            return None;
        }
        self.functions[i].monotone(range.map(|x| domain.map_f64(x, self.encode[i])))
    }

    fn find_interval(&self, x: f64) -> usize {
        let inner = &self.bounds[1..self.bounds.len() - 1];

        if inner.first() == Some(&self.bounds[0]) && x == f64::from(self.bounds[0]) {
            return 0;
        }

        inner.partition_point(|bound| f64::from(*bound) <= x)
    }

    pub(super) fn linear_pieces(
        &self,
        domain: [f64; 2],
        offsets: [f64; 2],
        budget: &mut usize,
        tolerance: Option<f32>,
        emit: &mut super::PieceEmitter<'_>,
    ) -> Option<()> {
        let [first, last] = domain;
        if !self.ordered_bounds {
            return None;
        }
        let low = first.min(last);
        let high = first.max(last);
        let begin = self.bounds.partition_point(|&b| f64::from(b) <= low) - 1;
        let end = self
            .bounds
            .partition_point(|&bound| f64::from(bound) < high);
        if end - begin > *budget {
            return None;
        }
        for (x, adjacent) in [(low, begin), (high, end - 1)] {
            let owner = self.find_interval(x);
            if owner != adjacent {
                let mut values = [[0.0; super::MAX_COMPONENTS]; 2];
                for (out, i) in values.iter_mut().zip([owner, adjacent]) {
                    let interval = Interval {
                        min: self.bounds[i],
                        max: self.bounds[i + 1],
                    };
                    self.functions[i].eval_piece(
                        interval.map_f64(x, self.encode[i]),
                        &mut out[..self.num_outputs()],
                        budget,
                    )?;
                }
                if values[0] != values[1] {
                    return None;
                }
            }
        }
        let offset = |x: f64| offsets[0] + (x - first) / (last - first) * (offsets[1] - offsets[0]);
        let mut previous_end = [0.; super::MAX_COMPONENTS];
        for k in begin..end {
            let i = if first < last {
                k
            } else {
                end - 1 - (k - begin)
            };
            let interval = Interval {
                min: self.bounds[i],
                max: self.bounds[i + 1],
            };
            if interval.min == interval.max {
                continue;
            }
            let clipped = [
                f64::from(interval.min).max(low),
                f64::from(interval.max).min(high),
            ];
            let ordered = if first < last {
                clipped
            } else {
                [clipped[1], clipped[0]]
            };
            let child_domain = ordered.map(|x| interval.map_f64(x, self.encode[i]));
            let mut at_boundary = first > last && k != begin;
            self.functions[i].ranged_pieces(
                child_domain,
                ordered.map(offset),
                budget,
                tolerance,
                &mut |ts, a, b, remaining| {
                    if at_boundary && a != &previous_end[..a.len()] {
                        return None;
                    }
                    at_boundary = false;
                    if first > last {
                        previous_end[..b.len()].copy_from_slice(b);
                    }
                    emit(ts, a, b, remaining)
                },
            )?;
        }
        Some(())
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
                let bound = self.encode[index].map(
                    child_bound,
                    Interval {
                        min: bounds_i_minus_1,
                        max: bounds_i,
                    },
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

#[cfg(test)]
mod tests {
    use super::*;
    use hayro_syntax::object::FromBytes;

    #[test]
    fn clamped_child_pieces_follow_traversal_and_spend_one_budget() {
        for encoding in ["-1 3", "3 -1", "-2 -1", "3 2"] {
            let dict = format!("<< /FunctionType 3 /Domain [0 1] /Functions [
                << /FunctionType 2 /Domain [0 1] /Range [.25 .75] /N 1 >>] /Bounds [] /Encode [{encoding}] >>");
            let function = Function::new(&Object::from_bytes(dict.as_bytes()).unwrap()).unwrap();
            let crate::function::FunctionType::Type3(stitch) = &function.0.function_type else {
                unreachable!()
            };
            let constant_cost = stitch.functions[0].0.eval_steps;
            let count = if matches!(encoding, "-1 3" | "3 -1") {
                3 + 2 * constant_cost
            } else {
                constant_cost
            };
            for domain in [[0., 1.], [1., 0.]] {
                let mut budget = count;
                let mut previous = 0.;
                function
                    .linear_pieces(
                        domain,
                        [0., 1.],
                        &mut budget,
                        Some(0.01),
                        &mut |[s, e], a, b| {
                            assert_eq!(s, previous);
                            assert!(s < e);
                            previous = e;
                            for (t, expected) in [(s, a[0]), (e, b[0])] {
                                let x = domain[0] + f64::from(t) * (domain[1] - domain[0]);
                                let mut actual = [0.];
                                function.eval_1d_f64(x, &mut actual)?;
                                assert_eq!(actual, [expected]);
                            }
                            Some(())
                        },
                    )
                    .unwrap();
                assert_eq!((previous, budget), (1., 0));
                assert!(
                    function
                        .linear_pieces(
                            domain,
                            [0., 1.],
                            &mut (count - 1),
                            Some(0.01),
                            &mut |_, _, _| Some(())
                        )
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn reversed_stitch_clamps_channels_with_a_shared_budget() {
        let dict = b"<< /FunctionType 3 /Domain [0 1] /Range [.25 .75 .5 1] /Functions [
            << /FunctionType 2 /Domain [0 1] /N 1 /C0 [0 1] /C1 [1 0] >>] /Bounds [] /Encode [1 0] >>";
        let function = Function::new(&Object::from_bytes(dict).unwrap()).unwrap();
        for mut budget in [3, 4] {
            let fits = budget == 4;
            let mut edges = Vec::new();
            let result = function.linear_pieces(
                [0.0, 1.0],
                [0.0, 1.0],
                &mut budget,
                Some(0.01),
                &mut |ts, a, b| {
                    for (t, color) in ts.into_iter().zip([a, b]) {
                        assert_eq!(color, function.eval_one(&[t]).unwrap());
                    }
                    edges.push(ts[1]);
                    Some(())
                },
            );
            assert_eq!(result.is_some(), fits);
            if fits {
                assert_eq!(budget, 0);
                assert_eq!(edges, [0.25, 0.5, 0.75, 1.0]);
            }
        }
    }

    #[test]
    fn nested_stitch_subsets_share_the_piece_budget() {
        let line = "<< /FunctionType 2 /Domain [0 1] /N 1 >>";
        let child = format!(
            "<< /FunctionType 3 /Domain [0 2] /Functions [{line} {line}] /Bounds [1] /Encode [0 1 1 0] >>"
        );
        let outer = format!(
            "<< /FunctionType 3 /Domain [-2 2] /Functions [{child} {child}] /Bounds [0] /Encode [0 2 0 2] >>"
        );
        let function = Function::new(&Object::from_bytes(outer.as_bytes()).unwrap()).unwrap();
        for domain in [[-1.5, 0.5], [0.5, -1.5]] {
            let mut pieces = Vec::new();
            let mut budget = 3;
            function
                .linear_pieces(
                    domain,
                    [0.0, 1.0],
                    &mut budget,
                    Some(0.01),
                    &mut |offsets, a, b| {
                        pieces.push((offsets, a[0], b[0]));
                        Some(())
                    },
                )
                .unwrap();
            assert_eq!(pieces.len(), 3);
            assert_eq!(budget, 0);
            for ([start, end], a, b) in pieces {
                for (offset, expected) in [(start, a), (end, b)] {
                    assert_eq!(
                        function
                            .eval_one(&[
                                (domain[0] + (domain[1] - domain[0]) * f64::from(offset)) as f32
                            ])
                            .unwrap()[0],
                        expected
                    );
                }
            }
            assert!(
                function
                    .linear_pieces(domain, [0.0, 1.0], &mut 2, Some(0.01), &mut |_, _, _| Some(
                        ()
                    ))
                    .is_none()
            );
        }
    }

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

    #[test]
    fn stitch_endpoint_uses_mapped_child_input_for_infinite_encode() {
        let data = b"<<
          /FunctionType 3
          /Domain [0 1]
          /Functions [
            << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [0] /N 1 >>
            << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>
          ]
          /Bounds [0.5]
          /Encode [0 1 0 10000000000000000000000000000000000000000.0]
        >>";
        let function = Function::new(&Object::from_bytes(data).unwrap()).unwrap();
        // At 0.5, the right child's infinite encode end yields NaN -> clamped to 1.0 (output 1.0 != 0.0).
        assert_eq!(function.eval_one(&[0.5]).unwrap().as_slice(), &[1.0]);
        let mut budget = 64;
        assert!(
            function
                .linear_pieces(
                    [0.0, 0.5],
                    [0.0, 1.0],
                    &mut budget,
                    Some(0.01),
                    &mut |_, _, _| { Some(()) }
                )
                .is_none()
        );
    }
}
