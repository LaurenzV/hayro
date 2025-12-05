use crate::function::{Clamper, Values};
use hayro_syntax::content;
use hayro_syntax::object::Number;
use hayro_syntax::object::Stream;
use hayro_syntax::reader::Reader;
use hayro_syntax::reader::{ReaderContext, ReaderExt};
use log::error;
use smallvec::SmallVec;
use std::array;
use std::ops::Rem;

/// A type 4 function (postscript function).
#[derive(Debug)]
pub(crate) struct Type4 {
    program: Vec<PostScriptOp>,
    clamper: Clamper,
}

impl Type4 {
    /// Create a new type 4 function.
    pub(crate) fn new(stream: &Stream<'_>) -> Option<Self> {
        let dict = stream.dict().clone();
        let clamper = Clamper::new(&dict)?;

        Some(Self {
            clamper,
            program: parse_procedure(&stream.decoded().ok()?)?,
        })
    }

    /// Evaluate the function with the given input.
    pub(crate) fn eval(&self, mut input: Values) -> Option<Values> {
        self.clamper.clamp_input(&mut input);

        let mut arg_stack = InterpreterStack::new();

        for input in input {
            arg_stack.push(Argument::Float(input));
        }

        eval_inner(&self.program, &mut arg_stack)?;

        let mut out: SmallVec<_> = arg_stack.items().iter().map(|i| i.as_f32()).collect();

        self.clamper.clamp_output(&mut out);

        Some(out)
    }
}

#[derive(Clone, Copy)]
enum Argument {
    Float(f32),
    Bool(bool),
}

impl Default for Argument {
    fn default() -> Self {
        Self::Float(0.0)
    }
}

impl Argument {
    fn as_bool(&self) -> bool {
        match self {
            Self::Float(f) => *f != 0.0,
            Self::Bool(b) => *b,
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Float(f) => *f,
            Self::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

struct ArgumentsStack<T: Default, const C: usize> {
    stack: [T; C],
    len: usize,
}

impl<T: Default, const C: usize> ArgumentsStack<T, C> {
    fn new() -> Self {
        Self {
            stack: array::from_fn(|_| T::default()),
            len: 0,
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.len
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    fn push(&mut self, n: T) -> Option<()> {
        if self.len == C {
            error!("overflowed post script argument stack");

            None
        } else {
            self.stack[self.len] = n;
            self.len += 1;

            Some(())
        }
    }

    #[inline]
    fn at(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            None
        } else {
            Some(&self.stack[index])
        }
    }

    #[inline]
    fn last(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            Some(&self.stack[self.len - 1])
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            error!("underflowed post script argument stack");

            None
        } else {
            self.len -= 1;
            Some(std::mem::take(&mut self.stack[self.len]))
        }
    }

    #[inline]
    fn items(&self) -> &[T] {
        &self.stack[..self.len]
    }

    #[inline]
    fn items_mut(&mut self) -> &mut [T] {
        &mut self.stack[..self.len]
    }
}

type InterpreterStack = ArgumentsStack<Argument, 64>;
type ParseStack = ArgumentsStack<Vec<PostScriptOp>, 2>;

fn eval_inner(procedure: &[PostScriptOp], arg_stack: &mut InterpreterStack) -> Option<()> {
    macro_rules! zero {
        ($eval:expr) => {
            arg_stack.push($eval);
        };
    }

    macro_rules! one_f {
        ($eval:expr) => {
            let n1 = arg_stack.pop()?;
            arg_stack.push(Argument::Float($eval(n1.as_f32())));
        };
    }

    macro_rules! two_f {
        ($eval:expr) => {
            let n2 = arg_stack.pop()?;
            let n1 = arg_stack.pop()?;
            arg_stack.push(Argument::Float($eval(n1.as_f32(), n2.as_f32())));
        };
    }

    macro_rules! four {
        ($eval_f:expr, $eval_b:expr) => {
            let n2 = arg_stack.pop()?;
            let n1 = arg_stack.pop()?;

            let res = match ((n1, n2)) {
                (Argument::Float(f1), Argument::Float(f2)) => Argument::Float($eval_f(f1, f2)),
                (Argument::Float(_), Argument::Bool(f2)) => {
                    Argument::Bool($eval_b(n1.as_bool(), f2))
                }
                (Argument::Bool(f1), Argument::Float(_)) => {
                    Argument::Bool($eval_b(f1, n2.as_bool()))
                }
                (Argument::Bool(f1), Argument::Bool(f2)) => Argument::Bool($eval_b(f1, f2)),
            };

            arg_stack.push(res);
        };
    }

    fn bf(cond: bool) -> f32 {
        (cond as i32) as f32
    }

    for op in procedure {
        match op {
            PostScriptOp::Number(n) => arg_stack.push(Argument::Float(n.as_f64() as f32))?,
            PostScriptOp::Abs => {
                one_f!(|n: f32| n.abs());
            }
            PostScriptOp::Add => {
                two_f!(|n1: f32, n2: f32| n1 + n2);
            }
            PostScriptOp::Atan => {
                two_f!(|n1: f32, n2: f32| {
                    let mut res = n1.atan2(n2).to_degrees() % 360.0;
                    if res < 0.0 {
                        res += 360.0;
                    }

                    res
                });
            }
            PostScriptOp::Ceiling => {
                one_f!(|n: f32| n.ceil());
            }
            PostScriptOp::Cos => {
                one_f!(|n: f32| n.to_radians().cos());
            }
            PostScriptOp::Cvi => {
                one_f!(|n: f32| n.trunc());
            }
            PostScriptOp::Cvr => {
                one_f!(|n: f32| n);
            }
            PostScriptOp::Div => {
                two_f!(|n1: f32, n2: f32| n1 / n2);
            }
            PostScriptOp::Exp => {
                two_f!(|n1: f32, n2: f32| n1.powf(n2));
            }
            PostScriptOp::Floor => {
                one_f!(|n: f32| n.floor());
            }
            PostScriptOp::Idiv => {
                two_f!(|n1: f32, n2: f32| {
                    let n1 = n1 as i32;
                    let n2 = n2 as i32;

                    (n1 / n2) as f32
                });
            }
            PostScriptOp::Ln => {
                one_f!(|n: f32| n.ln());
            }
            PostScriptOp::Log => {
                one_f!(|n: f32| n.log10());
            }
            PostScriptOp::Mod => {
                two_f!(|n1: f32, n2: f32| n1.rem(n2));
            }
            PostScriptOp::Mul => {
                two_f!(|n1: f32, n2: f32| n1 * n2);
            }
            PostScriptOp::Neg => {
                one_f!(|n: f32| -n);
            }
            PostScriptOp::Round => {
                one_f!(|n: f32| n.round());
            }
            PostScriptOp::Sin => {
                one_f!(|n: f32| n.to_radians().sin());
            }
            PostScriptOp::Sqrt => {
                one_f!(|n: f32| n.sqrt());
            }
            PostScriptOp::Sub => {
                two_f!(|n1: f32, n2: f32| n1 - n2);
            }
            PostScriptOp::Truncate => {
                one_f!(|n: f32| n.trunc());
            }
            PostScriptOp::And => {
                two_f!(|n1: f32, n2: f32| (n1 as i32 & n2 as i32) as f32);
            }
            PostScriptOp::Bitshift => {
                two_f!(|n1: f32, n2: f32| {
                    let num = n1 as u32;
                    let shift = n2 as i32;

                    if shift >= 0 {
                        (num << shift) as f32
                    } else {
                        (num >> -shift) as f32
                    }
                });
            }
            PostScriptOp::Eq => {
                two_f!(|n1: f32, n2: f32| bf(n1 == n2));
            }
            PostScriptOp::False => {
                zero!(Argument::Bool(false));
            }
            PostScriptOp::Ge => {
                two_f!(|n1: f32, n2: f32| bf(n1 >= n2));
            }
            PostScriptOp::Gt => {
                two_f!(|n1: f32, n2: f32| bf(n1 > n2));
            }
            PostScriptOp::Le => {
                two_f!(|n1: f32, n2: f32| bf(n1 <= n2));
            }
            PostScriptOp::Lt => {
                two_f!(|n1: f32, n2: f32| bf(n1 < n2));
            }
            PostScriptOp::Ne => {
                two_f!(|n1: f32, n2: f32| bf(n1 != n2));
            }
            PostScriptOp::Not => {
                let arg = arg_stack.pop()?;

                let res = match arg {
                    Argument::Float(f) => Argument::Float(!(f as i32) as f32),
                    Argument::Bool(b) => Argument::Bool(!b),
                };

                arg_stack.push(res);
            }
            PostScriptOp::Or => {
                four!(
                    |n1: f32, n2: f32| ((n1 as i32) | (n2 as i32)) as f32,
                    |b1: bool, b2: bool| b1 || b2
                );
            }
            PostScriptOp::True => {
                zero!(Argument::Bool(true));
            }
            PostScriptOp::Xor => {
                four!(
                    |n1: f32, n2: f32| ((n1 as i32) ^ (n2 as i32)) as f32,
                    |b1: bool, b2: bool| b1 ^ b2
                );
            }
            PostScriptOp::If(p) => {
                let cond = arg_stack.pop()?.as_bool();

                if cond {
                    eval_inner(p, arg_stack)?;
                }
            }
            PostScriptOp::IfElse(p1, p2) => {
                let cond = arg_stack.pop()?.as_bool();

                if cond {
                    eval_inner(p1, arg_stack)?;
                } else {
                    eval_inner(p2, arg_stack)?;
                }
            }
            PostScriptOp::Copy => {
                let n = arg_stack.pop()?.as_f32() as u32 as usize;
                let start = arg_stack.len().checked_sub(n)?;
                for i in start..arg_stack.len() {
                    arg_stack.push(*arg_stack.at(i)?);
                }
            }
            PostScriptOp::Dup => {
                arg_stack.push(*arg_stack.last()?);
            }
            PostScriptOp::Exch => {
                let n2 = arg_stack.pop()?;
                let n1 = arg_stack.pop()?;

                arg_stack.push(n2);
                arg_stack.push(n1);
            }
            PostScriptOp::Index => {
                let n = arg_stack.pop()?.as_f32() as u32 as usize;
                let n = arg_stack.len().checked_sub(n + 1)?;

                arg_stack.push(*arg_stack.at(n)?);
            }
            PostScriptOp::Pop => {
                arg_stack.pop()?;
            }
            PostScriptOp::Roll => {
                let j = arg_stack.pop()?.as_f32() as i32;
                let n = arg_stack.pop()?.as_f32() as u32 as usize;
                let trimmed_n = arg_stack.len().checked_sub(n)?;

                let target = &mut arg_stack.items_mut()[trimmed_n..];

                if target.is_empty() {
                    continue;
                }

                if j >= 0 {
                    let shift = j as usize % target.len();
                    target.rotate_right(shift);
                } else {
                    let shift = (-j) as usize % target.len();
                    target.rotate_left(shift);
                }
            }
        }
    }

    Some(())
}

fn parse_procedure(data: &[u8]) -> Option<Vec<PostScriptOp>> {
    let mut r = Reader::new(data);
    parse_procedure_inner(&mut r)
}

fn parse_procedure_inner(r: &mut Reader<'_>) -> Option<Vec<PostScriptOp>> {
    let mut stack = ParseStack::new();

    let mut ops = vec![];
    r.skip_white_spaces_and_comments();
    r.forward_tag(b"{")?;

    loop {
        r.skip_white_spaces_and_comments();

        if r.peek_byte()? == b'}' {
            r.forward_tag(b"}")?;

            break;
        } else if r.peek_byte()? == b'{' {
            stack.push(parse_procedure_inner(r)?);
        } else {
            let op = PostScriptOp::from_reader(r, &mut stack)?;
            ops.push(op);
        }
    }

    Some(ops)
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum PostScriptOp {
    Number(Number),
    Abs,
    Add,
    Atan,
    Ceiling,
    Cos,
    Cvi,
    Cvr,
    Div,
    Exp,
    Floor,
    Idiv,
    Ln,
    Log,
    Mod,
    Mul,
    Neg,
    Round,
    Sin,
    Sqrt,
    Sub,
    Truncate,
    And,
    Bitshift,
    Eq,
    False,
    Ge,
    Gt,
    Le,
    Lt,
    Ne,
    Not,
    Or,
    True,
    Xor,
    If(Vec<PostScriptOp>),
    IfElse(Vec<PostScriptOp>, Vec<PostScriptOp>),
    Copy,
    Dup,
    Exch,
    Index,
    Pop,
    Roll,
}

impl PostScriptOp {
    fn from_reader(r: &mut Reader<'_>, stack: &mut ParseStack) -> Option<Self> {
        let op = if let Some(n) = r.read::<Number>(&ReaderContext::dummy()) {
            // TODO: Support radix numbers
            Self::Number(n)
        } else {
            let op = r.read::<content::Operator<'_>>(&ReaderContext::dummy())?;
            match op.as_ref() {
                b"abs" => Self::Abs,
                b"add" => Self::Add,
                b"atan" => Self::Atan,
                b"ceiling" => Self::Ceiling,
                b"cos" => Self::Cos,
                b"cvi" => Self::Cvi,
                b"cvr" => Self::Cvr,
                b"div" => Self::Div,
                b"exp" => Self::Exp,
                b"floor" => Self::Floor,
                b"idiv" => Self::Idiv,
                b"ln" => Self::Ln,
                b"log" => Self::Log,
                b"mod" => Self::Mod,
                b"mul" => Self::Mul,
                b"neg" => Self::Neg,
                b"round" => Self::Round,
                b"sin" => Self::Sin,
                b"sqrt" => Self::Sqrt,
                b"sub" => Self::Sub,
                b"truncate" => Self::Truncate,
                b"and" => Self::And,
                b"bitshift" => Self::Bitshift,
                b"eq" => Self::Eq,
                b"false" => Self::False,
                b"ge" => Self::Ge,
                b"gt" => Self::Gt,
                b"le" => Self::Le,
                b"lt" => Self::Lt,
                b"ne" => Self::Ne,
                b"not" => Self::Not,
                b"or" => Self::Or,
                b"true" => Self::True,
                b"xor" => Self::Xor,
                b"if" => Self::If(stack.pop()?),
                b"ifelse" => {
                    let s = stack.pop()?;
                    let f = stack.pop()?;
                    Self::IfElse(f, s)
                }
                b"copy" => Self::Copy,
                b"dup" => Self::Dup,
                b"exch" => Self::Exch,
                b"index" => Self::Index,
                b"pop" => Self::Pop,
                b"roll" => Self::Roll,
                _ => {
                    error!("encountered unknown postscript operator {op:?}");

                    return None;
                }
            }
        };

        Some(op)
    }
}

#[cfg(test)]
mod tests {
    use crate::function::type4::{PostScriptOp, Type4, parse_procedure};
    use crate::function::{Clamper, Function, FunctionType, TupleVec, Values};
    use std::f32::consts::LN_10;
    use std::sync::Arc;

    use hayro_syntax::object::Number;
    use smallvec::smallvec;

    #[test]
    fn lex_1() {
        let program = b"{ copy dup 2.0 exch roll }";
        let parsed = parse_procedure(program).unwrap();

        assert_eq!(
            parsed,
            vec![
                PostScriptOp::Copy,
                PostScriptOp::Dup,
                PostScriptOp::Number(Number::from_i32(2)),
                PostScriptOp::Exch,
                PostScriptOp::Roll,
            ]
        );
    }

    #[test]
    fn lex_3() {
        let program = b" {  {dup exch} if {0} {1} ifelse }";
        let parsed = parse_procedure(program).unwrap();

        assert_eq!(
            parsed,
            vec![
                PostScriptOp::If(vec![PostScriptOp::Dup, PostScriptOp::Exch]),
                PostScriptOp::IfElse(
                    vec![PostScriptOp::Number(Number::from_i32(0))],
                    vec![PostScriptOp::Number(Number::from_i32(1))]
                )
            ]
        );
    }

    fn op_impl(prog: &str, out: &[f32]) {
        let procedure = format!("{{{prog}}}");
        let procedure = parse_procedure(procedure.as_bytes()).unwrap();

        let type4 = Type4 {
            program: procedure,
            clamper: Clamper {
                domain: TupleVec::default(),
                range: None,
            },
        };

        let res = type4.eval(Values::new()).unwrap();

        assert_eq!(res.as_slice(), out);
    }

    #[test]
    fn op_abs() {
        op_impl("4.5 abs", &[4.5]);
        op_impl("-3 abs", &[3.0]);
        op_impl("0 abs", &[0.0]);
    }

    #[test]
    fn op_atan() {
        op_impl("0 1 atan", &[0.0]);
        op_impl("1 0 atan", &[90.0]);
        op_impl("-100 0 atan", &[270.0]);
        op_impl("4 4 atan", &[45.0]);
    }

    #[test]
    fn op_ceiling() {
        op_impl("3.2 ceiling", &[4.0]);
        op_impl("-4.8 ceiling", &[-4.0]);
        op_impl("99 ceiling", &[99.0]);
    }

    #[test]
    fn op_cos() {
        op_impl("0 cos", &[1.0]);
        // Should be zero, but floating point impreciseness.
        op_impl("90 cos", &[-4.371139e-8]);
    }

    #[test]
    fn op_cvi() {
        op_impl("-47.8 cvi", &[-47.0]);
        op_impl("520.9 cvi", &[520.0]);
    }

    #[test]
    fn op_cvr() {
        op_impl("-47.8 cvr", &[-47.8]);
        op_impl("520 cvr", &[520.0]);
    }

    #[test]
    fn op_div() {
        op_impl("3 2 div", &[1.5]);
        op_impl("4 2 div", &[2.0]);
    }

    #[test]
    fn op_exp() {
        op_impl("9 0.5 exp", &[3.0]);
        op_impl("-9 -1 exp", &[-0.11111111]);
    }

    #[test]
    fn op_floor() {
        op_impl("3.2 floor", &[3.0]);
        op_impl("-4.8 floor", &[-5.0]);
        op_impl("99 floor", &[99.0]);
    }

    #[test]
    fn op_idiv() {
        op_impl("3 2 idiv", &[1.0]);
        op_impl("4 2 idiv", &[2.0]);
        op_impl("-5 2 idiv", &[-2.0]);
    }

    #[test]
    fn op_ln() {
        op_impl("10 ln", &[LN_10]);
        op_impl("100 ln", &[4.6051702]);
    }

    #[test]
    fn op_log() {
        op_impl("10 log", &[1.0]);
        op_impl("100 log", &[2.0]);
    }

    #[test]
    fn op_mod() {
        op_impl("5 3 mod", &[2.0]);
        op_impl("5 2 mod", &[1.0]);
        op_impl("-5 3 mod", &[-2.0]);
    }

    #[test]
    fn op_mul() {
        op_impl("5 3 mul", &[15.0]);
        op_impl("-2 6 mul", &[-12.0]);
    }

    #[test]
    fn op_neg() {
        op_impl("4.5 neg", &[-4.5]);
        op_impl("-3 neg", &[3.0]);
    }

    #[test]
    fn op_round() {
        op_impl("3.2 round", &[3.0]);
        op_impl("6.5 round", &[7.0]);
        op_impl("-4.8 round", &[-5.0]);
        // TODO: This rounding doesn't match the PS spec.
        // op_impl("-6.5 round", &[-6.0]);
        op_impl("99 round", &[99.0]);
    }

    #[test]
    fn op_sin() {
        op_impl("0.0 sin", &[0.0]);
        op_impl("90.0 sin", &[1.0]);
    }

    #[test]
    fn op_sqrt() {
        op_impl("100 sqrt", &[10.0]);
    }

    #[test]
    fn op_sub() {
        op_impl("3 4 sub", &[-1.0]);
        op_impl("6 0 sub", &[6.0]);
    }

    #[test]
    fn op_truncate() {
        op_impl("3.2 truncate", &[3.0]);
        op_impl("-4.8 truncate", &[-4.0]);
        op_impl("99 truncate", &[99.0]);
    }

    #[test]
    fn op_and() {
        op_impl("true true and", &[1.0]);
        op_impl("true false and", &[0.0]);
        op_impl("false true and", &[0.0]);
        op_impl("false false and", &[0.0]);
        op_impl("99 1 and", &[1.0]);
        op_impl("52 7 and", &[4.0]);
    }

    #[test]
    fn op_bitshift() {
        op_impl("7 3 bitshift", &[56.0]);
        op_impl("142 -3 bitshift", &[17.0]);
    }

    #[test]
    fn op_eq() {
        op_impl("4.0 4 eq", &[1.0]);
        op_impl("-2.0 -3 eq", &[0.0]);
    }

    #[test]
    fn op_false() {
        op_impl("false", &[0.0]);
    }

    #[test]
    fn op_ge() {
        op_impl("4.2 4 ge", &[1.0]);
        op_impl("4.2 4.2 ge", &[1.0]);
        op_impl("4.2 6 ge", &[0.0]);
    }

    #[test]
    fn op_gt() {
        op_impl("4.2 4 gt", &[1.0]);
        op_impl("4.2 4.2 gt", &[0.0]);
        op_impl("4.2 6 gt", &[0.0]);
    }

    #[test]
    fn op_le() {
        op_impl("4.2 4 le", &[0.0]);
        op_impl("4.2 4.2 le", &[1.0]);
        op_impl("4.2 6 le", &[1.0]);
    }

    #[test]
    fn op_lt() {
        op_impl("4.2 4 lt", &[0.0]);
        op_impl("4.2 4.2 lt", &[0.0]);
        op_impl("4.2 6 lt", &[1.0]);
    }

    #[test]
    fn op_ne() {
        op_impl("3.0 3 ne", &[0.0]);
        op_impl("3.0 3.0 ne", &[0.0]);
        op_impl("3.0 3.1 ne", &[1.0]);
    }

    #[test]
    fn op_not() {
        op_impl("true not", &[0.0]);
        op_impl("false not", &[1.0]);
        op_impl("52 not", &[-53.0]);
    }

    #[test]
    fn op_or() {
        op_impl("true true or", &[1.0]);
        op_impl("true false or", &[1.0]);
        op_impl("false true or", &[1.0]);
        op_impl("false false or", &[0.0]);
        op_impl("17 5 or", &[21.0]);
    }

    #[test]
    fn op_xor() {
        op_impl("true true xor", &[0.0]);
        op_impl("true false xor", &[1.0]);
        op_impl("false true xor", &[1.0]);
        op_impl("false false xor", &[0.0]);
        op_impl("7 3 xor", &[4.0]);
        op_impl("12 3 xor", &[15.0]);
    }

    #[test]
    fn op_if() {
        op_impl("true { 1.0 } if", &[1.0]);
        op_impl("false { 1.0 } if", &[]);
    }

    #[test]
    fn op_ifelse() {
        op_impl("true { 1.0 } { 2.0 } ifelse", &[1.0]);
        op_impl("false { 1.0 } { 2.0 } ifelse", &[2.0]);
    }

    #[test]
    fn op_copy() {
        op_impl("1 2 3 2 copy", &[1.0, 2.0, 3.0, 2.0, 3.0]);
        op_impl("1 2 3 0 copy", &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn op_dup() {
        op_impl("1 2 3 dup", &[1.0, 2.0, 3.0, 3.0]);
    }

    #[test]
    fn op_exch() {
        op_impl("1 2 3 exch", &[1.0, 3.0, 2.0]);
    }

    #[test]
    fn op_index() {
        op_impl("1 2 3 4 0 index", &[1.0, 2.0, 3.0, 4.0, 4.0]);
        op_impl("1 2 3 4 3 index", &[1.0, 2.0, 3.0, 4.0, 1.0]);
    }

    #[test]
    fn op_pop() {
        op_impl("1 2 3 pop", &[1.0, 2.0]);
    }

    #[test]
    fn op_roll() {
        op_impl("1 2 3 3 -1 roll", &[2.0, 3.0, 1.0]);
        op_impl("1 2 3 3 1 roll", &[3.0, 1.0, 2.0]);
        op_impl("1 2 3 3 0 roll", &[1.0, 2.0, 3.0]);
        op_impl("1 2 3 3 5 roll", &[2.0, 3.0, 1.0]);
        op_impl("0 2 roll", &[]);
        op_impl(
            "1 2 3 4 5 6 7 5 2 roll",
            &[1.0, 2.0, 6.0, 7.0, 3.0, 4.0, 5.0],
        );
    }

    #[test]
    fn domain() {
        let procedure = parse_procedure(b"{  }").unwrap();

        let type4 = Function(Arc::new(FunctionType::Type4(Type4 {
            program: procedure,
            clamper: Clamper {
                domain: smallvec![(-5.0, 5.0), (-5.0, 5.0), (-5.0, 5.0)],
                range: None,
            },
        })));

        let input = smallvec![-10.0, -2.0, 6.0];
        let res = type4.eval(input).unwrap();

        assert_eq!(res.as_slice(), &[-5.0, -2.0, 5.0]);
    }
}
