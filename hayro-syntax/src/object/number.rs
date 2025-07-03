//! Number objects.

use crate::object::macros::object;
use crate::object::{Object, ObjectLike};
use crate::reader::{Readable, Reader, ReaderContext, Skippable};
use log::debug;
use std::fmt::Debug;
use std::str::FromStr;

/// A PDF number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number(pub(crate) InternalNumber);

impl Number {
    /// Returns the number as a f64.
    pub fn as_f64(&self) -> f64 {
        match self.0 {
            InternalNumber::Real(r) => r as f64,
            InternalNumber::Integer(i) => i as f64,
        }
    }

    /// Returns the number as a f32.
    pub fn as_f32(&self) -> f32 {
        match self.0 {
            InternalNumber::Real(r) => r,
            InternalNumber::Integer(i) => {
                let converted = i as f32;

                // Double check whether conversion didn't overflow.
                if converted as i32 != i {
                    debug!("integer {} was truncated to {}", i, converted);
                }

                converted
            }
        }
    }

    /// Returns the number as an i32.
    pub fn as_i32(&self) -> i32 {
        match self.0 {
            InternalNumber::Real(r) => {
                let res = r as i32;

                if !(r.trunc() == r) {
                    debug!("float {} was truncated to {}", r, res);
                }

                res
            }
            InternalNumber::Integer(i) => i,
        }
    }

    /// Create a new `Number` from a f32 number.
    pub fn from_f32(num: f32) -> Self {
        Self(InternalNumber::Real(num))
    }

    /// Create a new `Number` from a i32 number.
    pub fn from_i32(num: i32) -> Self {
        Self(InternalNumber::Integer(num))
    }
}

impl Skippable for Number {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        r.forward_if(|b| b == b'+' || b == b'-');

        match r.peek_byte()? {
            b'.' => {
                r.read_byte()?;
                r.forward_while_1(is_digit)?;
            }
            (b'0'..=b'9') => {
                r.forward_while_1(is_digit)?;
                if let Some(()) = r.forward_tag(b".") {
                    r.forward_while(is_digit);
                }
            }
            _ => return None,
        }

        Some(())
    }
}

impl Readable<'_> for Number {
    fn read(r: &mut Reader<'_>, ctx: ReaderContext) -> Option<Self> {
        // TODO: This function is probably the biggest bottleneck in content parsing, so
        // worth optimizing (i.e. reading the number directly from the bytes instead
        // of first parsing it to a number).

        let data = r.skip::<Number>(ctx.in_content_stream)?;
        // We need to use f64 here, so that we can still parse a full `i32` without losing
        // precision.
        let num = f64::from_str(std::str::from_utf8(data).ok()?).ok()?;

        if num.fract() == 0.0 {
            Some(Number(InternalNumber::Integer(num as i32)))
        } else {
            Some(Number(InternalNumber::Real(num as f32)))
        }
    }
}

object!(Number, Number);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InternalNumber {
    Real(f32),
    Integer(i32),
}

macro_rules! int_num {
    ($i:ident) => {
        impl Skippable for $i {
            fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
                r.forward_if(|b| b == b'+' || b == b'-');
                r.forward_while_1(is_digit)?;

                // We have a float instead of an integer.
                if r.peek_byte() == Some(b'.') {
                    return None;
                }

                Some(())
            }
        }

        impl<'a> Readable<'a> for $i {
            fn read(r: &mut Reader<'a>, ctx: ReaderContext<'a>) -> Option<$i> {
                r.read::<Number>(ctx)
                    .map(|n| n.as_i32())
                    .and_then(|n| n.try_into().ok())
            }
        }

        impl TryFrom<Object<'_>> for $i {
            type Error = ();

            fn try_from(value: Object<'_>) -> std::result::Result<Self, Self::Error> {
                match value {
                    Object::Number(n) => n.as_i32().try_into().ok().ok_or(()),
                    _ => Err(()),
                }
            }
        }

        impl<'a> ObjectLike<'a> for $i {}
    };
}

int_num!(i32);
int_num!(u32);
int_num!(u16);
int_num!(usize);
int_num!(u8);

impl Skippable for f32 {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        r.skip::<Number>(is_content_stream).map(|_| {})
    }
}

impl Readable<'_> for f32 {
    fn read(r: &mut Reader, _: ReaderContext) -> Option<Self> {
        r.read_without_context::<Number>().map(|n| n.as_f32())
    }
}

impl TryFrom<Object<'_>> for f32 {
    type Error = ();

    fn try_from(value: Object<'_>) -> Result<Self, Self::Error> {
        match value {
            Object::Number(n) => Ok(n.as_f32()),
            _ => Err(()),
        }
    }
}

impl ObjectLike<'_> for f32 {}

impl Skippable for f64 {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        r.skip::<Number>(is_content_stream).map(|_| {})
    }
}

impl Readable<'_> for f64 {
    fn read(r: &mut Reader, _: ReaderContext) -> Option<Self> {
        r.read_without_context::<Number>().map(|n| n.as_f64())
    }
}

impl TryFrom<Object<'_>> for f64 {
    type Error = ();

    fn try_from(value: Object<'_>) -> Result<Self, Self::Error> {
        match value {
            Object::Number(n) => Ok(n.as_f64()),
            _ => Err(()),
        }
    }
}

impl ObjectLike<'_> for f64 {}

pub(crate) fn is_digit(byte: u8) -> bool {
    byte >= b'0' && byte <= b'9'
}

#[cfg(test)]
mod tests {
    use crate::object::number::Number;
    use crate::reader::Reader;

    #[test]
    fn int_1() {
        assert_eq!(
            Reader::new("0".as_bytes())
                .read_without_context::<i32>()
                .unwrap(),
            0
        );
    }

    #[test]
    fn int_3() {
        assert_eq!(
            Reader::new("+32".as_bytes())
                .read_without_context::<i32>()
                .unwrap(),
            32
        );
    }

    #[test]
    fn int_4() {
        assert_eq!(
            Reader::new("-32".as_bytes())
                .read_without_context::<i32>()
                .unwrap(),
            -32
        );
    }

    #[test]
    fn int_6() {
        assert_eq!(
            Reader::new("98349".as_bytes())
                .read_without_context::<i32>()
                .unwrap(),
            98349
        );
    }

    #[test]
    fn int_7() {
        assert_eq!(
            Reader::new("003245".as_bytes())
                .read_without_context::<i32>()
                .unwrap(),
            3245
        );
    }

    #[test]
    fn int_trailing() {
        assert_eq!(
            Reader::new("0abc".as_bytes())
                .read_without_context::<i32>()
                .unwrap(),
            0
        );
    }

    #[test]
    fn real_1() {
        assert_eq!(
            Reader::new("3".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            3.0
        );
    }

    #[test]
    fn real_3() {
        assert_eq!(
            Reader::new("+32".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            32.0
        );
    }

    #[test]
    fn real_4() {
        assert_eq!(
            Reader::new("-32".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            -32.0
        );
    }

    #[test]
    fn real_5() {
        assert_eq!(
            Reader::new("-32.01".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            -32.01
        );
    }

    #[test]
    fn real_6() {
        assert_eq!(
            Reader::new("-.345".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            -0.345
        );
    }

    #[test]
    fn real_7() {
        assert_eq!(
            Reader::new("-.00143".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            -0.00143
        );
    }

    #[test]
    fn real_8() {
        assert_eq!(
            Reader::new("-12.0013".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            -12.0013
        );
    }

    #[test]
    fn real_9() {
        assert_eq!(
            Reader::new("98349.432534".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            98349.432534
        );
    }

    #[test]
    fn real_10() {
        assert_eq!(
            Reader::new("-34534656.34".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            -34534656.34
        );
    }

    #[test]
    fn real_trailing() {
        assert_eq!(
            Reader::new("0abc".as_bytes())
                .read_without_context::<f32>()
                .unwrap(),
            0.0
        );
    }

    #[test]
    fn real_failing() {
        assert_eq!(
            Reader::new("+abc".as_bytes())
                .read_without_context::<f32>()
                .is_none(),
            true
        );
    }

    #[test]
    fn number_1() {
        assert_eq!(
            Reader::new("+32".as_bytes())
                .read_without_context::<Number>()
                .unwrap()
                .as_f64() as f32,
            32.0
        );
    }

    #[test]
    fn number_2() {
        assert_eq!(
            Reader::new("-32.01".as_bytes())
                .read_without_context::<Number>()
                .unwrap()
                .as_f64() as f32,
            -32.01
        );
    }

    #[test]
    fn number_3() {
        assert_eq!(
            Reader::new("-.345".as_bytes())
                .read_without_context::<Number>()
                .unwrap()
                .as_f64() as f32,
            -0.345
        );
    }

    #[test]
    fn large_number() {
        assert_eq!(
            Reader::new("38359922".as_bytes())
                .read_without_context::<Number>()
                .unwrap()
                .as_i32(),
            38359922
        );
    }
}
