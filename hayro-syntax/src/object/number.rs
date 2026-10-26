//! Numbers.

use crate::object::macros::object;
use crate::object::{Object, ObjectLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::{is_regular_character, is_white_space_character};
use core::fmt::{Debug, Display, Formatter};

// The powers of ten that an `f64` represents exactly.
#[rustfmt::skip]
static POWERS_OF_10: [f64; 23] = [
    1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9,
    1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19,
    1e20, 1e21, 1e22,
];

// The largest mantissa below which every integer converts to `f64` exactly.
const MAX_EXACT_MANTISSA: u64 = 1 << 53;

/// A number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number(pub(crate) InternalNumber);

impl Number {
    /// The number zero.
    pub const ZERO: Self = Self::from_i32(0);
    /// The number one.
    pub const ONE: Self = Self::from_i32(1);

    /// Returns the number as a f64.
    pub fn as_f64(&self) -> f64 {
        match self.0 {
            InternalNumber::Real(r) => r,
            InternalNumber::Integer(i) => i as f64,
        }
    }

    /// Returns the number as a f32.
    pub fn as_f32(&self) -> f32 {
        match self.0 {
            InternalNumber::Real(r) => r as f32,
            InternalNumber::Integer(i) => i as f32,
        }
    }

    /// Returns the number as an i64.
    pub fn as_i64(&self) -> i64 {
        match self.0 {
            InternalNumber::Real(r) => r as i64,
            InternalNumber::Integer(i) => i,
        }
    }

    /// Create a new `Number` from an f32 number.
    pub const fn from_f32(num: f32) -> Self {
        Self(InternalNumber::Real(num as f64))
    }

    /// Create a new `Number` from an i32 number.
    pub const fn from_i32(num: i32) -> Self {
        Self(InternalNumber::Integer(num as i64))
    }
}

impl Display for Number {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            InternalNumber::Real(value) => write!(f, "{value:?}"),
            InternalNumber::Integer(value) => write!(f, "{value}"),
        }
    }
}

impl Skippable for Number {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        let has_sign = r.forward_if(|b| b == b'+' || b == b'-').is_some();

        // Some PDFs have weird trailing minuses, so try to accept those as well.
        match r.peek_byte()? {
            b'.' => {
                r.read_byte()?;
                // See PDFJS-9252 - treat a single . as 0.
                r.forward_while(is_digit_or_minus);
            }
            b'0'..=b'9' | b'-' => {
                r.forward_while_1(is_digit_or_minus)?;
                if let Some(()) = r.forward_tag(b".") {
                    r.forward_while(is_digit_or_minus);
                }
            }
            // See PDFJS-bug1753983 - accept just + or - as a zero.
            // ALso see PDFJS-bug1953099, where the sign is followed by a show
            // text string operand, requiring us to allow '<' and '(' as well.
            b if has_sign && (is_white_space_character(b) || matches!(b, b'(' | b'<')) => {}
            _ => return None,
        }

        // See issue 994. Don't accept numbers that are followed by a regular character.
        if r.peek_byte().is_some_and(is_regular_character) {
            return None;
        }

        Some(())
    }
}

impl Readable<'_> for Number {
    #[inline]
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        let old_offset = r.offset();
        Self::read_simple(r)
            .or_else(|| {
                r.jump(old_offset);
                read_inner(r)
            })
            .or_else(|| {
                r.jump(old_offset);
                None
            })
    }
}

impl Number {
    #[inline(always)]
    pub(crate) fn read_simple(r: &mut Reader<'_>) -> Option<Self> {
        let negative = match r.peek_byte() {
            Some(b'-') => {
                r.forward();
                true
            }
            Some(b'+') => {
                r.forward();
                false
            }
            _ => false,
        };

        let mut mantissa: u64 = 0;
        let mut read_digits = |r: &mut Reader<'_>| {
            let start = r.offset();
            while let Some(d) = r
                .peek_byte()
                .map(|b| b.wrapping_sub(b'0'))
                .filter(|d| *d < 10)
            {
                mantissa = mantissa.wrapping_mul(10).wrapping_add(d as u64);
                r.forward();
            }
            r.offset() - start
        };

        let mut digits = read_digits(r);
        let mut decimal_shift = None;
        if r.peek_byte() == Some(b'.') {
            r.forward();
            let fraction_digits = read_digits(r);
            decimal_shift = Some(fraction_digits as u32);
            digits += fraction_digits;
        }

        if digits == 0 || digits > 18 || r.peek_byte().is_some_and(is_regular_character) {
            return None;
        }

        Self::from_parts(negative, mantissa, decimal_shift)
    }

    /// Returns `None` for a real that the fast path can't read exactly.
    #[inline(always)]
    fn from_parts(negative: bool, mantissa: u64, decimal_shift: Option<u32>) -> Option<Self> {
        match decimal_shift {
            None => {
                let value = mantissa as i64;
                Some(Self(InternalNumber::Integer(if negative {
                    value.wrapping_neg()
                } else {
                    value
                })))
            }
            Some(shift) => {
                // Clinger's fast path: when both the mantissa and the power of
                // ten are exact, the one division rounds once, so the result is
                // the correctly rounded `f64`.
                if mantissa > MAX_EXACT_MANTISSA {
                    return None;
                }
                let divisor = *POWERS_OF_10.get(shift as usize)?;
                let value = mantissa as f64 / divisor;
                Some(Self(InternalNumber::Real(if negative {
                    -value
                } else {
                    value
                })))
            }
        }
    }
}

#[inline(always)]
fn read_inner(r: &mut Reader<'_>) -> Option<Number> {
    let start_offset = r.offset();
    let negative = match r.peek_byte()? {
        b'-' => {
            r.forward();
            true
        }
        b'+' => {
            r.forward();
            false
        }
        _ => false,
    };

    let mut mantissa: u64 = 0;
    let mut has_dot = false;
    let mut decimal_shift: u32 = 0;
    let mut has_digits = false;
    let mut overflowed = false;
    let mut number_end = None;

    loop {
        match r.peek_byte() {
            Some(b'0'..=b'9') => {
                let d = r.read_byte().unwrap();
                if !overflowed {
                    if let Some(value) = mantissa
                        .checked_mul(10)
                        .and_then(|v| v.checked_add((d - b'0') as u64))
                    {
                        mantissa = value;
                    } else {
                        overflowed = true;
                    }
                }
                has_digits = true;
                if has_dot {
                    decimal_shift += 1;
                }
            }
            Some(b'.') if !has_dot => {
                r.forward();
                has_dot = true;
            }
            // Some weird PDFs have trailing minus in the fraction of number.
            Some(b'-') if has_digits => {
                number_end = Some(r.offset());
                r.forward();
                r.forward_while(is_digit_or_minus);
                break;
            }
            _ => break,
        }
    }

    if !has_digits {
        if negative || has_dot {
            // Treat numbers like just `-`, `+` or `-.` as zero.
            return Some(Number(InternalNumber::Integer(0)));
        }
        return None;
    }

    // See issue 994. Don't accept numbers that are followed by a regular character
    // without any white space in-between.
    if r.peek_byte().is_some_and(is_regular_character) {
        return None;
    }

    let integer_overflow = !has_dot && mantissa > i64::MAX as u64;

    if !overflowed
        && !integer_overflow
        && let Some(number) =
            Number::from_parts(negative, mantissa, has_dot.then_some(decimal_shift))
    {
        return Some(number);
    }

    // The number is too long for the fast path, so read it through the
    // standard library, which rounds correctly.
    let end_offset = number_end.unwrap_or_else(|| r.offset());
    let number = core::str::from_utf8(r.range(start_offset..end_offset)?).ok()?;

    if !has_dot && let Ok(value) = number.parse::<i64>() {
        return Some(Number(InternalNumber::Integer(value)));
    }

    let value = number.parse::<f64>().ok()?;
    Some(Number(InternalNumber::Real(value)))
}

object!(Number, Number);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InternalNumber {
    Real(f64),
    Integer(i64),
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

                // See issue 994. Don't accept numbers that are followed by a regular character
                // without any white space in-between.
                if r.peek_byte().is_some_and(is_regular_character) {
                    return None;
                }

                Some(())
            }
        }

        impl<'a> Readable<'a> for $i {
            fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<$i> {
                r.read::<Number>(ctx)
                    .map(|n| n.as_i64())
                    .and_then(|n| n.try_into().ok())
            }
        }

        impl TryFrom<Object<'_>> for $i {
            type Error = ();

            fn try_from(value: Object<'_>) -> core::result::Result<Self, Self::Error> {
                match value {
                    Object::Number(n) => n.as_i64().try_into().ok().ok_or(()),
                    _ => Err(()),
                }
            }
        }

        impl<'a> ObjectLike<'a> for $i {}
    };
}

int_num!(i32);
int_num!(i64);
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
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        r.read_without_context::<Number>()
            .map(|n| n.as_f64() as Self)
    }
}

impl TryFrom<Object<'_>> for f32 {
    type Error = ();

    fn try_from(value: Object<'_>) -> Result<Self, Self::Error> {
        match value {
            Object::Number(n) => Ok(n.as_f64() as Self),
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
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
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
    byte.is_ascii_digit()
}

pub(crate) fn is_digit_or_minus(byte: u8) -> bool {
    is_digit(byte) || byte == b'-'
}

#[cfg(test)]
mod tests {
    use crate::object::Number;
    use crate::reader::Reader;
    use crate::reader::ReaderExt;

    #[test]
    fn display() {
        assert_eq!(format!("{}", Number::from_i32(10)), "10");
        assert_eq!(format!("{}", Number::from_f32(10.0)), "10.0");
        assert_eq!(format!("{}", Number::from_f32(10.5)), "10.5");
    }

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
    fn int_min_stays_integer() {
        let number = Reader::new("-9223372036854775808".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        assert_eq!(number.0, super::InternalNumber::Integer(i64::MIN));
        assert_eq!(number.as_i64(), i64::MIN);
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
            98_349.43
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
    fn real_failing() {
        assert!(
            Reader::new("+abc".as_bytes())
                .read_without_context::<f32>()
                .is_none()
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
                .as_i64(),
            38359922
        );
    }

    #[test]
    fn large_number_2() {
        assert_eq!(
            Reader::new("4294966260".as_bytes())
                .read_without_context::<u32>()
                .unwrap(),
            4294966260
        );
    }

    #[test]
    fn twenty_digit_real_does_not_wrap() {
        let got = Reader::new("190.50000762939453225".as_bytes())
            .read_without_context::<Number>()
            .unwrap()
            .as_f64();
        assert_eq!(got, 190.500_007_629_394_53);
    }

    #[test]
    fn nineteen_digit_real_stays_exact() {
        let got = Reader::new("190.5000076293945313".as_bytes())
            .read_without_context::<Number>()
            .unwrap()
            .as_f64();
        assert_eq!(got, 190.500_007_629_394_53);
    }

    #[test]
    fn oversized_integer_uses_float_fallback() {
        let n = Reader::new("9999999999999999999".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        assert_eq!(n.as_f64(), 9_999_999_999_999_999_999_u64 as f64);
        assert_eq!(n.as_i64(), i64::MAX);
    }

    #[test]
    fn oversized_negative_integer_uses_float_fallback() {
        let n = Reader::new("-1234567890123456789012345".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        assert_eq!(n.as_f64(), -1.234_567_890_123_456_8e24);
        assert_eq!(n.as_i64(), i64::MIN);
    }

    #[test]
    fn integer_overflow_then_fraction_uses_float_fallback() {
        let got = Reader::new("12345678901234567890123.456".as_bytes())
            .read_without_context::<Number>()
            .unwrap()
            .as_f64();
        assert_eq!(got, 1.234_567_890_123_456_8e22);
    }

    #[test]
    fn f32_max_uses_float_fallback() {
        let got = Reader::new("340282346638528859811704183484516925440".as_bytes())
            .read_without_context::<f32>()
            .unwrap();
        assert_eq!(got, f32::MAX);
    }

    #[test]
    fn float_fallback_ignores_trailing_minus() {
        let input = "1234567890123456789012345-123".as_bytes();
        let mut reader = Reader::new(input);
        let got = reader.read_without_context::<Number>().unwrap().as_f64();
        assert_eq!(got, 1.234_567_890_123_456_8e24);
        assert_eq!(reader.offset(), input.len());
    }

    fn read_f64(input: &str) -> f64 {
        let mut reader = Reader::new(input.as_bytes());
        let got = reader.read_without_context::<Number>().unwrap().as_f64();
        assert_eq!(reader.offset(), input.len());
        got
    }

    fn read_f64_from_content(input: &str) -> f64 {
        let content = format!("{input} 0 Td");
        let mut ops = crate::content::UntypedIter::new(content.as_bytes());
        let op = ops.next().unwrap();
        match op.operands().next() {
            Some(crate::object::Object::Number(number)) => number.as_f64(),
            other => panic!("{other:?}"),
        }
    }

    // Each value is written with Rust's `{}`, the shortest decimal that a
    // correctly rounded parser reads back to the same `f64`.
    #[test]
    fn reals_round_correctly() {
        for value in [
            // A mantissa above 2^53: rounding it to `f64` and then dividing
            // rounds twice.
            519_188.221_071_372_04,
            // A power of ten above 10^22 isn't exact.
            1e-25,
            // The smallest normal and subnormal numbers.
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            0.1,
            1.5,
            -0.000_123,
            9_007_199_254_740_993.0,
            core::f64::consts::PI,
        ] {
            let text = format!("{value}");
            assert_eq!(read_f64(&text).to_bits(), value.to_bits(), "{text}");
            assert_eq!(
                read_f64_from_content(&text).to_bits(),
                value.to_bits(),
                "{text}"
            );
        }
    }

    #[test]
    fn real_at_fast_path_limits() {
        // 2^53 with a fraction: the largest mantissa that the fast path takes.
        assert_eq!(read_f64("900719925474099.2"), 900_719_925_474_099.2);
        // 2^53 + 1 doesn't fit and goes through the exact parser.
        assert_eq!(read_f64("900719925474099.3"), 900_719_925_474_099.3);
        // 10^22 is the largest exact power of ten; 10^23 isn't.
        assert_eq!(read_f64("0.0000000000000000000001"), 1e-22);
        assert_eq!(read_f64("0.00000000000000000000001"), 1e-23);
    }

    #[test]
    fn real_fallback_keeps_trailing_minus_and_sign() {
        assert_eq!(read_f64("+519188.22107137204"), 519_188.221_071_372_04);
        assert_eq!(read_f64("-519188.22107137204-12"), -519_188.221_071_372_04);
    }
}
