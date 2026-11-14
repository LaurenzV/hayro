use hayro_postscript::{Procedure, Scanner, Token};
use hayro_syntax::object::Stream;

const MAX_PROCEDURE_DEPTH: usize = 256;
const MAX_STACK_DEPTH: usize = 100;

#[derive(Debug)]
pub(crate) struct Type4 {
    code: Vec<PostScriptOp>,
}

impl Type4 {
    pub(crate) fn new(stream: &Stream<'_>) -> Option<Self> {
        Some(Self {
            code: parse_procedure(&stream.decoded_data().ok()?)?,
        })
    }

    pub(crate) fn evaluator(&self) -> impl FnMut(&[f32], &mut [f32]) -> Option<()> + '_ {
        let mut stack = Stack::new();

        move |input, output| {
            let len = input.len();
            stack.values.get_mut(..len)?.copy_from_slice(input);
            stack.bools[..len].fill(false);
            stack.len = len;

            execute(&self.code, &mut stack)?;

            let values = &stack.values[..stack.len];
            output.copy_from_slice(values.get(values.len().checked_sub(output.len())?..)?);

            Some(())
        }
    }
}

struct Stack {
    values: [f32; MAX_STACK_DEPTH],
    bools: [bool; MAX_STACK_DEPTH],
    len: usize,
}

impl Stack {
    fn new() -> Self {
        Self {
            values: [0.0; MAX_STACK_DEPTH],
            bools: [false; MAX_STACK_DEPTH],
            len: 0,
        }
    }
}

fn execute(code: &[PostScriptOp], stack: &mut Stack) -> Option<()> {
    use PostScriptOp as Op;

    let Stack { values, bools, .. } = stack;
    let mut len = stack.len;

    macro_rules! pop {
        () => {{
            len = len.checked_sub(1)?;
            values[len]
        }};
    }

    macro_rules! push {
        ($value:expr, $bool:expr) => {{
            let value = $value;
            *values.get_mut(len)? = value;
            bools[len] = $bool;
            len += 1;
        }};
    }

    macro_rules! unary {
        ($f:expr) => {{
            let i = len.checked_sub(1)?;
            values[i] = $f(values[i]);
            bools[i] = false;
        }};
    }

    macro_rules! binary {
        ($f:expr, $bool:expr) => {{
            let i = len.checked_sub(2)?;
            values[i] = $f(values[i], values[i + 1])?;
            bools[i] = $bool;
            len -= 1;
        }};
    }

    macro_rules! number {
        ($f:expr) => {
            binary!(|a: f32, b: f32| Some($f(a, b)), false)
        };
    }

    macro_rules! compare {
        ($f:expr) => {
            binary!(|a: f32, b: f32| Some(f32::from($f(a, b))), true)
        };
    }

    macro_rules! logical {
        ($int:expr, $bool:expr) => {{
            let i = len.checked_sub(2)?;
            let any_bool = bools[i] || bools[i + 1];
            let (a, b) = (values[i], values[i + 1]);

            values[i] = if any_bool {
                f32::from($bool(a != 0.0, b != 0.0))
            } else {
                $int(a as i32, b as i32) as f32
            };
            bools[i] = any_bool;
            len -= 1;
        }};
    }

    let mut pc = 0;

    while let Some(&op) = code.get(pc) {
        pc += 1;

        match op {
            Op::Number(n) => push!(n, false),
            Op::True => push!(1.0, true),
            Op::False => push!(0.0, true),
            Op::Abs => unary!(f32::abs),
            Op::Ceiling => unary!(f32::ceil),
            Op::Cos => unary!(|n: f32| n.to_radians().cos()),
            Op::Cvi | Op::Truncate => unary!(f32::trunc),
            Op::Cvr => unary!(|n| n),
            Op::Floor => unary!(f32::floor),
            Op::Ln => unary!(f32::ln),
            Op::Log => unary!(f32::log10),
            Op::Neg => unary!(|n: f32| -n),
            Op::Round => unary!(f32::round),
            Op::Sin => unary!(|n: f32| n.to_radians().sin()),
            Op::Sqrt => unary!(f32::sqrt),
            Op::Add => number!(|a, b| a + b),
            Op::Atan => number!(|a: f32, b| {
                let degrees = a.atan2(b).to_degrees();
                if degrees < 0.0 {
                    degrees + 360.0
                } else {
                    degrees
                }
            }),
            Op::Div => number!(|a, b| a / b),
            Op::Exp => number!(f32::powf),
            Op::Idiv => binary!(
                |a: f32, b: f32| Some((a as i32).checked_div(b as i32)? as f32),
                false
            ),
            Op::Mod => number!(|a, b| a % b),
            Op::Mul => number!(|a, b| a * b),
            Op::Sub => number!(|a, b| a - b),
            Op::Bitshift => number!(|a: f32, b: f32| {
                let (num, shift) = (a as u32, b as i32);

                if shift >= 0 {
                    num.checked_shl(shift as u32).unwrap_or(0) as f32
                } else {
                    num.checked_shr(shift.unsigned_abs()).unwrap_or(0) as f32
                }
            }),
            Op::Eq => compare!(|a, b| a == b),
            Op::Ge => compare!(|a, b| a >= b),
            Op::Gt => compare!(|a, b| a > b),
            Op::Le => compare!(|a, b| a <= b),
            Op::Lt => compare!(|a, b| a < b),
            Op::Ne => compare!(|a, b| a != b),
            Op::And => logical!(|a, b| a & b, |a, b| a && b),
            Op::Or => logical!(|a, b| a | b, |a, b| a || b),
            Op::Xor => logical!(|a, b| a ^ b, |a, b| a ^ b),
            Op::Not => {
                let i = len.checked_sub(1)?;
                values[i] = if bools[i] {
                    f32::from(values[i] == 0.0)
                } else {
                    !(values[i] as i32) as f32
                };
            }
            Op::Copy => {
                let n = pop!() as u32 as usize;
                let start = len.checked_sub(n)?;

                for i in start..start + n {
                    push!(values[i], bools[i]);
                }
            }
            Op::Dup => {
                let i = len.checked_sub(1)?;
                push!(values[i], bools[i]);
            }
            Op::Exch => {
                let i = len.checked_sub(2)?;
                values.swap(i, i + 1);
                bools.swap(i, i + 1);
            }
            Op::Index => {
                let n = pop!() as u32 as usize;
                let i = len.checked_sub(n.checked_add(1)?)?;
                push!(values[i], bools[i]);
            }
            Op::Pop => {
                pop!();
            }
            Op::Roll => {
                let j = pop!() as i32;
                let n = pop!() as u32 as usize;
                let start = len.checked_sub(n)?;

                if n > 0 {
                    let shift = j.unsigned_abs() as usize % n;
                    let (values, bools) = (&mut values[start..len], &mut bools[start..len]);

                    if j >= 0 {
                        values.rotate_right(shift);
                        bools.rotate_right(shift);
                    } else {
                        values.rotate_left(shift);
                        bools.rotate_left(shift);
                    }
                }
            }
            Op::SkipUnless(n) => {
                if pop!() == 0.0 {
                    pc += n as usize;
                }
            }
            Op::Skip(n) => pc += n as usize,
        }
    }

    stack.len = len;

    Some(())
}

fn parse_procedure(data: &[u8]) -> Option<Vec<PostScriptOp>> {
    let procedure = Scanner::new(data).parse_procedure().ok()?;
    parse_procedure_inner(procedure, 0)
}

fn parse_procedure_inner(procedure: Procedure<'_>, depth: usize) -> Option<Vec<PostScriptOp>> {
    if depth >= MAX_PROCEDURE_DEPTH {
        return None;
    }

    let mut procedures: Vec<Vec<PostScriptOp>> = vec![];
    let mut code = vec![];
    let mut tokens = procedure.tokens();

    while !tokens.at_end() {
        match tokens.parse_token().ok()? {
            Token::Number(n) => code.push(PostScriptOp::Number(n.as_f64() as f32)),
            Token::Procedure(p) if procedures.len() < 2 => {
                procedures.push(parse_procedure_inner(p, depth + 1)?);
            }
            Token::Name(n) if !n.is_literal() && !n.is_immediate() => match n.as_bytes() {
                b"if" => {
                    let then = procedures.pop()?;
                    code.push(PostScriptOp::SkipUnless(then.len() as u32));
                    code.extend(then);
                }
                b"ifelse" => {
                    let otherwise = procedures.pop()?;
                    let then = procedures.pop()?;
                    code.push(PostScriptOp::SkipUnless(then.len() as u32 + 1));
                    code.extend(then);
                    code.push(PostScriptOp::Skip(otherwise.len() as u32));
                    code.extend(otherwise);
                }
                name => code.push(PostScriptOp::from_operator(name)?),
            },
            _ => return None,
        }
    }

    Some(code)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum PostScriptOp {
    Number(f32),
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
    Copy,
    Dup,
    Exch,
    Index,
    Pop,
    Roll,
    SkipUnless(u32),
    Skip(u32),
}

impl PostScriptOp {
    fn from_operator(op: &[u8]) -> Option<Self> {
        let op = match op {
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
            b"copy" => Self::Copy,
            b"dup" => Self::Dup,
            b"exch" => Self::Exch,
            b"index" => Self::Index,
            b"pop" => Self::Pop,
            b"roll" => Self::Roll,
            _ => return None,
        };

        Some(op)
    }
}

#[cfg(test)]
mod tests {
    use crate::function::type4::{
        MAX_PROCEDURE_DEPTH, PostScriptOp, Stack, Type4, execute, parse_procedure,
    };
    use crate::function::{Function, FunctionType, Interval, Repr};
    use std::f32::consts::LN_10;
    use std::sync::Arc;

    fn nested_program(depth: usize) -> Vec<u8> {
        let mut program = "{ true ".repeat(depth - 1);
        program.push_str("{ 1 }");
        program.push_str(&" if }".repeat(depth - 1));

        program.into_bytes()
    }

    #[test]
    fn nested_procedures_up_to_bound() {
        assert!(parse_procedure(&nested_program(MAX_PROCEDURE_DEPTH)).is_some());
        assert!(parse_procedure(&nested_program(MAX_PROCEDURE_DEPTH + 1)).is_none());
    }

    #[test]
    fn deep_procedure_nesting_fails() {
        let program = format!("{}{}", "{".repeat(300_000), "}".repeat(300_000));
        assert!(parse_procedure(program.as_bytes()).is_none());
    }

    #[test]
    fn lex_1() {
        let program = b"{ copy dup 2.0 exch roll }";
        let parsed = parse_procedure(program).unwrap();

        assert_eq!(
            parsed,
            vec![
                PostScriptOp::Copy,
                PostScriptOp::Dup,
                PostScriptOp::Number(2.0),
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
                PostScriptOp::SkipUnless(2),
                PostScriptOp::Dup,
                PostScriptOp::Exch,
                PostScriptOp::SkipUnless(2),
                PostScriptOp::Number(0.0),
                PostScriptOp::Skip(1),
                PostScriptOp::Number(1.0),
            ]
        );
    }

    fn run(code: &[PostScriptOp]) -> Option<Vec<f32>> {
        let mut stack = Stack::new();
        execute(code, &mut stack)?;

        Some(stack.values[..stack.len].to_vec())
    }

    fn op_impl(prog: &str, out: &[f32]) {
        let procedure = format!("{{{prog}}}");
        let procedure = parse_procedure(procedure.as_bytes()).unwrap();

        assert_eq!(run(&procedure).unwrap().as_slice(), out);
    }

    #[test]
    fn radix_numbers() {
        op_impl("16#FF 2#101", &[255.0, 5.0]);
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
        // TODO: `-6.5 round` should yield -6 per the PostScript spec.
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
    fn comparisons_are_booleans() {
        op_impl("1 2 lt not", &[0.0]);
        op_impl("2 1 lt not", &[1.0]);
        op_impl("1 2 lt 3 4 lt and not", &[0.0]);
    }

    #[test]
    fn op_idiv_by_zero() {
        let program = parse_procedure(b"{ 1 0 idiv }").unwrap();

        assert!(run(&program).is_none());
    }

    #[test]
    fn op_bitshift_out_of_range() {
        op_impl("1 40 bitshift", &[0.0]);
        op_impl("1 -40 bitshift", &[0.0]);
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

        let type4 = Function(Arc::new(Repr {
            domain: vec![
                Interval {
                    min: -5.0,
                    max: 5.0
                };
                3
            ],
            range: None,
            num_outputs: 3,
            function_type: FunctionType::Type4(Type4 { code: procedure }),
        }));

        let res = type4.eval_one(&[-10.0, -2.0, 6.0]).unwrap();

        assert_eq!(res.as_slice(), &[-5.0, -2.0, 5.0]);
    }
}
