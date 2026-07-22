//! Content stream operators.

use crate::content::{Instruction, OPERANDS_THRESHOLD, OperatorTrait, Stack};
use crate::object;
use crate::object::Array;
use crate::object::Name;
use crate::object::Number;
use crate::object::Object;
use crate::object::Stream;
use core::fmt::{Display, Formatter};
use smallvec::{SmallVec, smallvec};

use crate::content::macros::{op_all, op_impl, op0, op1, op2, op3, op4, op6};

include!("ops_generated.rs");

// Need to special-case those because they have variable arguments.

fn parse_named_color<'b, 'a>(
    objects: &'b [Object<'a>],
) -> Option<(SmallVec<[Number; OPERANDS_THRESHOLD]>, Option<&'b Name<'a>>)> {
    let mut nums = smallvec![];
    let mut name = None;

    for o in objects {
        match o {
            Object::Number(n) => nums.push(*n),
            Object::Name(n) => name = Some(n),
            _ => {
                warn!("encountered unknown object {o:?} when parsing scn/SCN");

                return None;
            }
        }
    }

    Some((nums, name))
}

fn fmt_named_color(
    f: &mut Formatter<'_>,
    operator: &str,
    numbers: &[Number],
    name: Option<&Name<'_>>,
) -> core::fmt::Result {
    f.write_str(operator)?;
    if !numbers.is_empty() || name.is_some() {
        f.write_str("(")?;
        for (index, number) in numbers.iter().enumerate() {
            if index > 0 {
                f.write_str(" ")?;
            }
            Display::fmt(number, f)?;
        }
        if let Some(name) = name {
            if !numbers.is_empty() {
                f.write_str(" ")?;
            }
            Display::fmt(name, f)?;
        }
        f.write_str(")")?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct StrokeColorNamed<'b, 'a>(
    pub SmallVec<[Number; OPERANDS_THRESHOLD]>,
    pub Option<&'b Name<'a>>,
);

impl Display for StrokeColorNamed<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        fmt_named_color(f, "StrokeColorNamed", &self.0, self.1)
    }
}

op_impl!(
    StrokeColorNamed<'b, 'a>,
    "SCN",
    u8::MAX as usize,
    |stack: &'b Stack<'a>| {
        let (nums, name) = parse_named_color(stack.as_slice())?;
        Some(StrokeColorNamed(nums, name))
    }
);

#[derive(Debug, PartialEq, Clone)]
pub struct NonStrokeColorNamed<'b, 'a>(
    pub SmallVec<[Number; OPERANDS_THRESHOLD]>,
    pub Option<&'b Name<'a>>,
);

impl Display for NonStrokeColorNamed<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        fmt_named_color(f, "NonStrokeColorNamed", &self.0, self.1)
    }
}

op_impl!(
    NonStrokeColorNamed<'b, 'a>,
    "scn",
    u8::MAX as usize,
    |stack: &'b Stack<'a>| {
        let (nums, name) = parse_named_color(stack.as_slice())?;
        Some(NonStrokeColorNamed(nums, name))
    }
);

#[cfg(test)]
mod tests {
    use crate::content::TypedIter;
    use crate::content::ops::{
        BeginMarkedContentWithProperties, ClosePath, EndMarkedContent, FillPathNonZero, LineTo,
        MarkedContentPointWithProperties, MoveTo, NonStrokeColorDeviceRgb, NonStrokeColorNamed,
        SetGraphicsState, StrokeColorNamed, Transform, TypedInstruction,
    };
    use crate::object::Name;
    use crate::object::Number;
    use crate::object::Object;
    use crate::object::{Dict, FromBytes};
    fn n(num: i32) -> Number {
        Number::from_i32(num)
    }

    #[test]
    fn basic_ops_1() {
        let input = b"
1 0 0 -1 0 200 cm
/g0 gs
1 0 0 rg
";

        let mut iter = TypedIter::new(input);

        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::Transform(Transform(a, b, c, d, e, f)))
                if [a, b, c, d, e, f] == [n(1), n(0), n(0), n(-1), n(0), n(200)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::SetGraphicsState(SetGraphicsState(name)))
                if name.as_ref() == b"g0"
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::NonStrokeColorDeviceRgb(NonStrokeColorDeviceRgb(r, g, b)))
                if [r, g, b] == [n(1), n(0), n(0)]
        ));
        assert!(iter.next().is_none());
    }

    #[test]
    fn basic_ops_2() {
        let input = b"
20 20 m
180 20 l
180 180 l
20 180 l
h
f
";

        let mut iter = TypedIter::new(input);

        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::MoveTo(MoveTo(x, y))) if [x, y] == [n(20), n(20)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::LineTo(LineTo(x, y))) if [x, y] == [n(180), n(20)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::LineTo(LineTo(x, y))) if [x, y] == [n(180), n(180)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::LineTo(LineTo(x, y))) if [x, y] == [n(20), n(180)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::ClosePath(ClosePath))
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::FillPathNonZero(FillPathNonZero))
        ));
        assert!(iter.next().is_none());
    }

    #[test]
    fn display_ops() {
        let mut iter = TypedIter::new(
            b"h 2 w 1 0 0 RG 0 0 10 20 re 10 20 m 30 20 60 40 90 80 c \
              0.1 0.2 SC 0.0 scn 1.0 /Spot SCN (Hello) Tj <00FF> Tj foo",
        );

        assert_eq!(format!("{}", iter.next().unwrap()), "ClosePath");
        assert_eq!(format!("{}", iter.next().unwrap()), "LineWidth(2)");
        assert_eq!(
            format!("{}", iter.next().unwrap()),
            "StrokeColorDeviceRgb(1 0 0)"
        );
        assert_eq!(format!("{}", iter.next().unwrap()), "RectPath(0 0 10 20)");
        assert_eq!(format!("{}", iter.next().unwrap()), "MoveTo(10 20)");
        assert_eq!(
            format!("{}", iter.next().unwrap()),
            "CubicTo(30 20 60 40 90 80)"
        );
        assert_eq!(format!("{}", iter.next().unwrap()), "StrokeColor(0.1 0.2)");
        assert_eq!(
            format!("{}", iter.next().unwrap()),
            "NonStrokeColorNamed(0.0)"
        );
        assert_eq!(
            format!("{}", iter.next().unwrap()),
            "StrokeColorNamed(1.0 /Spot)"
        );
        assert_eq!(format!("{}", iter.next().unwrap()), "ShowText(\"Hello\")");
        assert_eq!(format!("{}", iter.next().unwrap()), "ShowText([0, 255])");
        assert_eq!(format!("{}", iter.next().unwrap()), "Fallback(foo)");
        assert!(iter.next().is_none());
    }

    #[test]
    fn scn() {
        let input = b"
0.0 scn
1.0 1.0 1.0 /DeviceRgb SCN
";

        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::NonStrokeColorNamed(NonStrokeColorNamed(nums, None))) => {
                assert_eq!(nums.as_slice(), &[Number::from_f32(0.0)]);
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        match iter.next() {
            Some(TypedInstruction::StrokeColorNamed(StrokeColorNamed(nums, Some(name)))) => {
                assert_eq!(
                    nums.as_slice(),
                    &[
                        Number::from_f32(1.0),
                        Number::from_f32(1.0),
                        Number::from_f32(1.0)
                    ]
                );
                assert_eq!(name.as_ref(), b"DeviceRgb");
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        assert!(iter.next().is_none());
    }

    #[test]
    fn dp() {
        let input = b"/Attribute<</ShowCenterPoint false >> DP";

        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::MarkedContentPointWithProperties(
                MarkedContentPointWithProperties(name, object),
            )) => {
                assert_eq!(name.as_ref(), b"Attribute");
                assert_eq!(
                    object,
                    &Object::Dict(Dict::from_bytes(b"<</ShowCenterPoint false >>").unwrap())
                );
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        assert!(iter.next().is_none());
    }

    #[test]
    fn bdc_with_dict() {
        let input = b"/Span << /MCID 0 /Alt (Alt)>> BDC EMC";

        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::BeginMarkedContentWithProperties(
                BeginMarkedContentWithProperties(name, object),
            )) => {
                assert_eq!(name.as_ref(), b"Span");
                assert_eq!(
                    object,
                    &Object::Dict(Dict::from_bytes(b"<< /MCID 0 /Alt (Alt)>>").unwrap())
                );
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::EndMarkedContent(EndMarkedContent))
        ));
        assert!(iter.next().is_none());
    }

    #[test]
    fn bdc_with_name() {
        let input = b"/Span /Name BDC EMC";

        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::BeginMarkedContentWithProperties(
                BeginMarkedContentWithProperties(name, object),
            )) => {
                assert_eq!(name.as_ref(), b"Span");
                assert_eq!(object, &Object::Name(Name::new_unescaped(b"Name")));
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::EndMarkedContent(EndMarkedContent))
        ));
        assert!(iter.next().is_none());
    }
}
