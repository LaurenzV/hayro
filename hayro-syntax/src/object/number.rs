//! Numbers.

use crate::math::{powi_f64, trunc_f64};
use crate::object::macros::object;
use crate::object::{Object, ObjectLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::{is_regular_character, is_white_space_character};
use core::fmt::{Debug, Display, Formatter};

#[rustfmt::skip]
static POWERS_OF_10: [f64; 20] = [
    1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9,
    1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19,
];

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
            InternalNumber::Real(r) => {
                let res = r as i64;

                if !(trunc_f64(r) == r) {
                    debug!("float {r} was truncated to {res}");
                }

                res
            }
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
        read_inner(r).or_else(|| {
            r.jump(old_offset);
            None
        })
    }
}

#[inline(always)]
fn read_inner(r: &mut Reader<'_>) -> Option<Number> {
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
    // The number of integer digits that were dropped by the overflow guard
    // below. Each one scales the final value by 10 after the loop.
    let mut dropped_int_digits: u32 = 0;

    loop {
        match r.peek_byte() {
            Some(b'0'..=b'9') => {
                let d = r.read_byte().unwrap();
                // Stop accumulating once another digit could overflow the
                // mantissa. At that point the mantissa already carries at
                // least 19 significant digits, so a dropped digit perturbs
                // the value by a relative error of less than 1/mantissa
                // <= 5.5e-19, below the precision of an f64. Wrapping
                // instead would turn the whole mantissa into garbage, e.g.
                // `190.50000762939453225` (a full-precision f64 emitted
                // with 20 significant digits) used to parse as ~6.03.
                //
                // A dropped fractional digit must not bump `decimal_shift`
                // (the retained digits keep their scale), while a dropped
                // integer digit scales the result by 10, applied after the
                // loop. Note that the digit is consumed either way, so the
                // extent of the number stays in sync with `Skippable`.
                if mantissa <= (u64::MAX - 9) / 10 {
                    mantissa = mantissa * 10 + (d - b'0') as u64;
                    if has_dot {
                        decimal_shift += 1;
                    }
                } else if !has_dot {
                    dropped_int_digits += 1;
                }
                has_digits = true;
            }
            Some(b'.') if !has_dot => {
                r.forward();
                has_dot = true;
            }
            // Some weird PDFs have trailing minus in the fraction of number.
            Some(b'-') if has_digits => {
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

    // If integer digits were dropped by the overflow guard, the exact value
    // is not representable as an i64, so return a scaled `Real` instead.
    // Dropping only starts once the mantissa is saturated and it never
    // un-saturates, so when any integer digit was dropped every later
    // fractional digit was dropped as well and `decimal_shift` is still 0.
    // `as_i64` on such a huge `Real` is still well-behaved: float-to-int
    // `as` casts saturate, so typed integer readers see `i64::MAX`/
    // `i64::MIN` and reject the value cleanly via `try_into` instead of
    // receiving a wrapped bit pattern.
    if dropped_int_digits > 0 {
        let mut value = (mantissa as f64) * powi_f64(10.0, dropped_int_digits);

        if negative {
            value = -value;
        }

        return Some(Number(InternalNumber::Real(value)));
    }

    if !has_dot {
        // A mantissa in `(i64::MAX, u64::MAX]` fits the u64 accumulator
        // without tripping the digit guard above, but does not fit an i64:
        // the plain `as i64` cast below used to sign-wrap it (e.g.
        // `9999999999999999999` read back as -8446744073709551617). Route
        // this band to a `Real` as well; the f64 is within 1 ULP of the
        // exact value and `as_i64` saturates as described above. The
        // single value in the band that is exactly representable stays an
        // `Integer`: -(2^63) == `i64::MIN`.
        if mantissa > i64::MAX as u64 {
            if negative && mantissa == (i64::MAX as u64) + 1 {
                return Some(Number(InternalNumber::Integer(i64::MIN)));
            }

            let mut value = mantissa as f64;

            if negative {
                value = -value;
            }

            return Some(Number(InternalNumber::Real(value)));
        }

        let value = if negative {
            // Cannot wrap: the band above filtered out mantissa >
            // i64::MAX, so -(mantissa as i64) >= i64::MIN + 1.
            (mantissa as i64).wrapping_neg()
        } else {
            mantissa as i64
        };
        Some(Number(InternalNumber::Integer(value)))
    } else {
        let mut value = mantissa as f64;

        if decimal_shift > 0 {
            if decimal_shift < POWERS_OF_10.len() as u32 {
                value /= POWERS_OF_10[decimal_shift as usize];
            } else {
                value /= powi_f64(10.0, decimal_shift);
            }
        }

        if negative {
            value = -value;
        }

        Some(Number(InternalNumber::Real(value)))
    }
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
    fn int_min_does_not_panic() {
        assert_eq!(
            Reader::new("-9223372036854775808".as_bytes())
                .read_without_context::<i64>()
                .unwrap(),
            i64::MIN
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

    // Mantissas past 19 significant digits must degrade by dropping
    // trailing digits (a relative error below f64 precision), never by
    // wrapping the accumulator.

    #[test]
    fn twenty_digit_real_does_not_wrap() {
        // 19050000762939453225 mod 2^64 = 603256689229901609, so this
        // used to parse as ~6.0325668922.
        let got = Reader::new("190.50000762939453225".as_bytes())
            .read_without_context::<Number>()
            .unwrap()
            .as_f64();
        assert!((got - 190.50000762939453).abs() < 1e-9, "got {got}");
    }

    #[test]
    fn twenty_digit_real_negative() {
        let got = Reader::new("-190.50000762939453225".as_bytes())
            .read_without_context::<Number>()
            .unwrap()
            .as_f64();
        assert!((got + 190.50000762939453).abs() < 1e-9, "got {got}");
    }

    // A 19-digit mantissa still accumulates exactly: the guard only fires
    // on the digit that could overflow.
    #[test]
    fn nineteen_digit_real_stays_exact() {
        let got = Reader::new("190.5000076293945313".as_bytes())
            .read_without_context::<Number>()
            .unwrap()
            .as_f64();
        // == 190.50000762939453125 exactly, the halfway point between two
        // adjacent f32 values.
        assert_eq!(got, 190.500_007_629_394_53);
    }

    #[test]
    fn very_long_fraction_is_correct_to_f64_precision() {
        // 39 fractional digits of pi; the digits past the 19-significant-
        // digit mantissa are dropped.
        let got = Reader::new("3.141592653589793238462643383279502884197".as_bytes())
            .read_without_context::<Number>()
            .unwrap()
            .as_f64();
        assert!((got - core::f64::consts::PI).abs() < 1e-15, "got {got}");
    }

    #[test]
    fn twenty_five_digit_integer_scales_instead_of_wrapping() {
        let n = Reader::new("1234567890123456789012345".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        let got = n.as_f64();
        let want = 1.234_567_890_123_456_8e24;
        assert!(((got - want) / want).abs() < 1e-9, "got {got}");
        // Float-to-int casts saturate, so the typed integer readers see
        // i64::MAX instead of a wrapped bit pattern.
        assert_eq!(n.as_i64(), i64::MAX);
    }

    #[test]
    fn twenty_five_digit_integer_negative() {
        let n = Reader::new("-1234567890123456789012345".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        let got = n.as_f64();
        let want = -1.234_567_890_123_456_8e24;
        assert!(((got - want) / want).abs() < 1e-9, "got {got}");
        assert_eq!(n.as_i64(), i64::MIN);
    }

    // Overflowing digits before the dot and more digits after it: the
    // fractional digits arrive with the mantissa already saturated, so
    // they are all dropped and no decimal shift applies.
    #[test]
    fn integer_overflow_then_fraction() {
        let got = Reader::new("12345678901234567890123.456".as_bytes())
            .read_without_context::<Number>()
            .unwrap()
            .as_f64();
        let want = 1.234_567_890_123_456_8e22;
        assert!(((got - want) / want).abs() < 1e-9, "got {got}");
    }

    // The integer band (i64::MAX, u64::MAX] fits the u64 accumulator
    // without tripping the digit guard, but not an i64: it must surface
    // as a positive `Real` (typed readers then saturate at i64::MAX),
    // never as a sign-wrapped `Integer`.
    #[test]
    fn nineteen_digit_integer_above_i64_max_does_not_sign_wrap() {
        let n = Reader::new("9999999999999999999".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        assert_eq!(n.as_f64(), 9_999_999_999_999_999_999_u64 as f64);
        assert_eq!(n.as_i64(), i64::MAX);
    }

    #[test]
    fn two_pow_63_saturates_positive_and_stays_exact_negative() {
        let n = Reader::new("9223372036854775808".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        assert_eq!(n.as_f64(), 9223372036854775808.0);
        assert_eq!(n.as_i64(), i64::MAX);
        // The one exactly representable exception: -(2^63) is i64::MIN
        // (`int_min_does_not_panic` pins the typed read of the same
        // bytes).
        let m = Reader::new("-9223372036854775808".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        assert_eq!(m.as_i64(), i64::MIN);
    }

    #[test]
    fn max_reachable_mantissa_stays_positive() {
        // 18446744073709551609 is the accumulator's maximum reachable
        // value: the guard admits a digit while mantissa <=
        // 1844674407370955160, and the largest admitted digit is 9.
        let n = Reader::new("18446744073709551609".as_bytes())
            .read_without_context::<Number>()
            .unwrap();
        assert_eq!(n.as_f64(), 18_446_744_073_709_551_609_u64 as f64);
        assert_eq!(n.as_i64(), i64::MAX);
    }
}
