//! Content stream operators.

use crate::content::{Instruction, OPERANDS_THRESHOLD, OperatorTrait, Stack};
use crate::object;
use crate::object::Array;
use crate::object::Name;
use crate::object::Number;
use crate::object::Object;
use crate::object::Stream;
use smallvec::{SmallVec, smallvec};

use crate::content::macros::{op_all, op_impl, op0, op1, op2, op3, op4, op6};
use log::warn;

include!("ops_generated.rs");

// Need to special-case those because they have variable arguments.

#[derive(Debug, Clone, PartialEq)]
pub struct StrokeColorNamed<'a>(
    pub SmallVec<[Number; OPERANDS_THRESHOLD]>,
    pub Option<Name<'a>>,
);

fn scn_fn<'a>(
    stack: &Stack<'a>,
) -> Option<(SmallVec<[Number; OPERANDS_THRESHOLD]>, Option<Name<'a>>)> {
    let mut nums = smallvec![];
    let mut name = None;

    for o in &stack.0 {
        match o {
            Object::Number(n) => nums.push(*n),
            Object::Name(n) => name = Some(n.clone()),
            _ => {
                warn!("encountered unknown object {:?} when parsing scn/SCN", o);

                return None;
            }
        }
    }

    Some((nums, name))
}

op_impl!(
    StrokeColorNamed<'a>,
    "SCN",
    u8::MAX as usize,
    |stack: &Stack<'a>| {
        let res = scn_fn(stack);
        res.map(|r| StrokeColorNamed(r.0, r.1))
    }
);

#[derive(Debug, PartialEq, Clone)]
pub struct NonStrokeColorNamed<'a>(
    pub SmallVec<[Number; OPERANDS_THRESHOLD]>,
    pub Option<Name<'a>>,
);

op_impl!(
    NonStrokeColorNamed<'a>,
    "scn",
    u8::MAX as usize,
    |stack: &Stack<'a>| {
        let res = scn_fn(stack);
        res.map(|r| NonStrokeColorNamed(r.0, r.1))
    }
);

#[cfg(test)]
mod tests {
    use crate::content::ops::{
        BeginMarkedContentWithProperties, ClosePath, EndMarkedContent, FillPathNonZero, LineTo,
        MarkedContentPointWithProperties, MoveTo, NonStrokeColorDeviceRgb, NonStrokeColorNamed,
        SetGraphicsState, StrokeColorNamed, Transform, TypedInstruction,
    };
    use crate::content::{TypedIter, UntypedIter};
    use crate::object::Dict;
    use crate::object::Name;
    use crate::object::Number;
    use crate::object::Object;
    use crate::reader::Readable;
    use smallvec::smallvec;

    fn n(num: i32) -> Number {
        Number::from_i32(num)
    }

    fn f(num: f32) -> Number {
        Number::from_f32(num)
    }

    #[test]
    fn basic_ops_1() {
        let input = b"
1 0 0 -1 0 200 cm
/g0 gs
1 0 0 rg
";

        let expected = vec![
            TypedInstruction::Transform(Transform(n(1), n(0), n(0), n(-1), n(0), n(200))),
            TypedInstruction::SetGraphicsState(SetGraphicsState(Name::new(b"g0"))),
            TypedInstruction::NonStrokeColorDeviceRgb(NonStrokeColorDeviceRgb(n(1), n(0), n(0))),
        ];

        let elements = TypedIter::new(input).into_iter().collect::<Vec<_>>();
        assert_eq!(elements, expected,)
    }

    #[test]
    fn basic_ops_2() {
        let input = b"
20 20 m
180 20 l
180.1 180.1 l
20 180 l
h
f
";

        let expected = vec![
            TypedInstruction::MoveTo(MoveTo(n(20), n(20))),
            TypedInstruction::LineTo(LineTo(n(180), n(20))),
            TypedInstruction::LineTo(LineTo(f(180.1), f(180.1))),
            TypedInstruction::LineTo(LineTo(n(20), n(180))),
            TypedInstruction::ClosePath(ClosePath),
            TypedInstruction::FillPathNonZero(FillPathNonZero),
        ];

        let elements = TypedIter::new(input).into_iter().collect::<Vec<_>>();
        assert_eq!(elements, expected,)
    }

    #[test]
    fn scn() {
        let input = b"
0.0 scn
0.1 0.2 0.3 /DeviceRgb SCN
";

        let expected = vec![
            TypedInstruction::NonStrokeColorNamed(NonStrokeColorNamed(
                smallvec![Number::from_i32(0)],
                None,
            )),
            TypedInstruction::StrokeColorNamed(StrokeColorNamed(
                smallvec![
                    Number::from_f32(0.1),
                    Number::from_f32(0.2),
                    Number::from_f32(0.3)
                ],
                Some(Name::new(b"DeviceRgb")),
            )),
        ];

        let elements = TypedIter::new(input).into_iter().collect::<Vec<_>>();

        assert_eq!(elements, expected);
    }

    #[test]
    fn dp() {
        let input = b"/Attribute<</ShowCenterPoint false >> DP";

        let expected = vec![TypedInstruction::MarkedContentPointWithProperties(
            MarkedContentPointWithProperties(
                Name::new(b"Attribute"),
                Object::Dict(Dict::from_bytes(b"<</ShowCenterPoint false >>").unwrap()),
            ),
        )];

        let elements = TypedIter::new(input).into_iter().collect::<Vec<_>>();

        assert_eq!(elements, expected);
    }

    #[test]
    fn bdc_with_dict() {
        let input = b"/Span << /MCID 0 /Alt (Alt)>> BDC EMC";

        let expected = vec![
            TypedInstruction::BeginMarkedContentWithProperties(BeginMarkedContentWithProperties(
                Name::new(b"Span"),
                Object::Dict(Dict::from_bytes(b"<< /MCID 0 /Alt (Alt)>>").unwrap()),
            )),
            TypedInstruction::EndMarkedContent(EndMarkedContent),
        ];

        let elements = TypedIter::new(input).into_iter().collect::<Vec<_>>();

        assert_eq!(elements, expected);
    }

    #[test]
    fn bdc_with_name() {
        let input = b"/Span /Name BDC EMC";

        let expected = vec![
            TypedInstruction::BeginMarkedContentWithProperties(BeginMarkedContentWithProperties(
                Name::new(b"Span"),
                Object::Name(Name::new(b"Name")),
            )),
            TypedInstruction::EndMarkedContent(EndMarkedContent),
        ];

        let elements = TypedIter::new(input).into_iter().collect::<Vec<_>>();

        assert_eq!(elements, expected);
    }
}
