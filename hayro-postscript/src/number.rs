use crate::error::{Error, Result};
use crate::reader::{Reader, is_delimiter, is_whitespace};
use core::num::IntErrorKind;

/// A PostScript number object (integer or real).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Integer(i64),
    Real(f64),
}

impl Number {
    /// Return the value as an `f64`.
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Integer(v) => v as f64,
            Self::Real(v) => v,
        }
    }
}

fn is_terminated(r: &Reader<'_>) -> bool {
    match r.peek_byte() {
        None => true,
        Some(b) => is_whitespace(b) || is_delimiter(b),
    }
}

pub(crate) fn read(r: &mut Reader<'_>) -> Result<Number> {
    let saved = r.offset();

    // Optional sign.
    let first = r.peek_byte().ok_or(Error::SyntaxError)?;
    let has_sign = first == b'+' || first == b'-';

    if has_sign {
        r.forward();
    }

    // Consume leading digits.
    let digit_start = r.offset();
    r.forward_while(|b| b.is_ascii_digit());
    let has_digits = r.offset() > digit_start;

    // Check if number is a radix number.
    if !has_sign && has_digits && r.peek_byte() == Some(b'#') {
        let base_bytes = r.range(digit_start..r.offset()).ok_or(Error::SyntaxError)?;
        let base_str = core::str::from_utf8(base_bytes).map_err(|_| Error::SyntaxError)?;
        let base = base_str.parse::<u32>().map_err(|_| Error::SyntaxError)?;

        if !(2..=36).contains(&base) {
            return Err(Error::SyntaxError);
        }

        // Skip `#`.
        r.forward();

        let num_start = r.offset();
        r.forward_while(|b| b.is_ascii_alphanumeric());

        if r.offset() == num_start || !is_terminated(r) {
            return Err(Error::SyntaxError);
        }

        let num_bytes = r.range(num_start..r.offset()).ok_or(Error::SyntaxError)?;
        let num_str = core::str::from_utf8(num_bytes).map_err(|_| Error::SyntaxError)?;
        let value = u64::from_str_radix(num_str, base).map_err(|e| match e.kind() {
            IntErrorKind::PosOverflow => Error::LimitCheck,
            _ => Error::SyntaxError,
        })?;

        return Ok(Number::Integer(value as i64));
    }

    // Check for real number indicators: `.` or `e`/`E`.
    let has_dot = r.peek_byte() == Some(b'.');

    if has_dot {
        r.forward(); // skip '.'
        r.forward_while(|b| b.is_ascii_digit());
    }

    // At this point we need at least some digits (before or after the dot).
    if !has_digits && !has_dot {
        return Err(Error::SyntaxError);
    }

    let has_exponent = matches!(r.peek_byte(), Some(b'e' | b'E'));
    if has_exponent {
        r.forward();

        // Optional exponent sign.
        if matches!(r.peek_byte(), Some(b'+' | b'-')) {
            r.forward();
        }

        r.forward_while(|b| b.is_ascii_digit());
    }

    if !is_terminated(r) {
        return Err(Error::SyntaxError);
    }

    let token = r.range(saved..r.offset()).ok_or(Error::SyntaxError)?;
    let str = core::str::from_utf8(token).map_err(|_| Error::SyntaxError)?;

    if has_dot || has_exponent {
        real(str)
    } else {
        match str.parse::<i64>() {
            Ok(value) => Ok(Number::Integer(value)),
            Err(e)
                if matches!(
                    e.kind(),
                    IntErrorKind::PosOverflow | IntErrorKind::NegOverflow
                ) =>
            {
                real(str)
            }
            Err(_) => Err(Error::SyntaxError),
        }
    }
}

fn real(str: &str) -> Result<Number> {
    let value = str.parse::<f64>().map_err(|_| Error::SyntaxError)?;

    if value.is_finite() {
        Ok(Number::Real(value))
    } else {
        Err(Error::LimitCheck)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_num(input: &[u8]) -> Result<Number> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    #[test]
    fn signed_integers() {
        assert_eq!(read_num(b"123 ").unwrap(), Number::Integer(123));
        assert_eq!(read_num(b"-98 ").unwrap(), Number::Integer(-98));
        assert_eq!(read_num(b"43445 ").unwrap(), Number::Integer(43445));
        assert_eq!(read_num(b"0 ").unwrap(), Number::Integer(0));
        assert_eq!(read_num(b"+17 ").unwrap(), Number::Integer(17));
    }

    #[test]
    fn real_numbers() {
        assert_eq!(read_num(b"-.002 ").unwrap(), Number::Real(-0.002));
        assert_eq!(read_num(b"34.5 ").unwrap(), Number::Real(34.5));
        assert_eq!(read_num(b"-3.62 ").unwrap(), Number::Real(-3.62));
        assert_eq!(read_num(b"123.6e10 ").unwrap(), Number::Real(123.6e10));
        assert_eq!(read_num(b"1.0E-5 ").unwrap(), Number::Real(1.0E-5));
        assert_eq!(read_num(b"1E6 ").unwrap(), Number::Real(1E6));
        assert_eq!(read_num(b"-1. ").unwrap(), Number::Real(-1.0));
        assert_eq!(read_num(b"0.0 ").unwrap(), Number::Real(0.0));
        assert_eq!(read_num(b"1.e5 ").unwrap(), Number::Real(1e5));
    }

    #[test]
    fn radix_numbers() {
        assert_eq!(read_num(b"8#1777 ").unwrap(), Number::Integer(0o1777));
        assert_eq!(read_num(b"16#FFFE ").unwrap(), Number::Integer(0xFFFE));
        assert_eq!(read_num(b"2#1000 ").unwrap(), Number::Integer(0b1000));
        assert_eq!(read_num(b"36#ZZ ").unwrap(), Number::Integer(1295));
    }

    #[test]
    fn large_integers() {
        assert_eq!(
            read_num(b"4000000001 ").unwrap(),
            Number::Integer(4000000001)
        );
        assert_eq!(
            read_num(b"9223372036854775807 ").unwrap(),
            Number::Integer(i64::MAX)
        );
        assert_eq!(
            read_num(b"-9223372036854775808 ").unwrap(),
            Number::Integer(i64::MIN)
        );
    }

    #[test]
    fn integer_overflow_becomes_real() {
        assert_eq!(
            read_num(b"9223372036854775808 ").unwrap(),
            Number::Real(9223372036854775808.0)
        );
        assert_eq!(
            read_num(b"-9223372036854775809 ").unwrap(),
            Number::Real(-9223372036854775809.0)
        );
        assert_eq!(
            read_num(b"99999999999999999999999 ").unwrap(),
            Number::Real(1e23)
        );
    }

    #[test]
    fn real_overflow() {
        assert_eq!(read_num(b"1e400 "), Err(Error::LimitCheck));
        assert_eq!(read_num(b"-1e400 "), Err(Error::LimitCheck));
        assert_eq!(
            read_num(b"9".repeat(400).as_slice()),
            Err(Error::LimitCheck)
        );
    }

    #[test]
    fn radix_twos_complement() {
        assert_eq!(
            read_num(b"16#FFFFFFFF ").unwrap(),
            Number::Integer(0xFFFFFFFF)
        );
        assert_eq!(
            read_num(b"16#FFFFFFFFFFFFFFFF ").unwrap(),
            Number::Integer(-1)
        );
        assert_eq!(
            read_num(b"16#8000000000000000 ").unwrap(),
            Number::Integer(i64::MIN)
        );
    }

    #[test]
    fn radix_overflow() {
        assert_eq!(read_num(b"16#10000000000000000 "), Err(Error::LimitCheck));
        assert_eq!(read_num(b"8#9 "), Err(Error::SyntaxError));
    }

    #[test]
    fn invalid() {
        assert!(read_num(b"abc").is_err());
        assert!(read_num(b"+abc").is_err());
        assert!(read_num(b"1a").is_err());
        assert!(read_num(b"- ").is_err());
        assert!(read_num(b". ").is_err());
        assert!(read_num(b"1e ").is_err());
        assert!(read_num(b"-16#FF ").is_err());
    }
}
