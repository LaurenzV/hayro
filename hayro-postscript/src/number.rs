use crate::error::{Error, Result};
use crate::reader::{Reader, is_delimiter, is_whitespace};

/// A PostScript number object (integer or real).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Integer(i32),
    Real(f32),
}

impl Number {
    /// Return the value as an `i32`. Reals are truncated.
    pub fn as_i32(self) -> i32 {
        match self {
            Self::Integer(v) => v,
            Self::Real(v) => v as i32,
        }
    }

    /// Return the value as an `f32`.
    pub fn as_f32(self) -> f32 {
        match self {
            Self::Integer(v) => v as f32,
            Self::Real(v) => v,
        }
    }

    /// Return the value as an `f64`.
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Integer(v) => v as f64,
            Self::Real(v) => v as f64,
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
        let value = i32::from_str_radix(num_str, base).map_err(|_| Error::SyntaxError)?;

        return Ok(Number::Integer(value));
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
        let value = str.parse::<f32>().map_err(|_| Error::SyntaxError)?;

        Ok(Number::Real(value))
    } else {
        if !has_digits {
            return Err(Error::SyntaxError);
        }

        let value = str.parse::<i32>().map_err(|_| Error::SyntaxError)?;

        Ok(Number::Integer(value))
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
    }

    #[test]
    fn radix_numbers() {
        assert_eq!(read_num(b"8#1777 ").unwrap(), Number::Integer(0o1777));
        assert_eq!(read_num(b"16#FFFE ").unwrap(), Number::Integer(0xFFFE));
        assert_eq!(read_num(b"2#1000 ").unwrap(), Number::Integer(0b1000));
    }

    #[test]
    fn invalid() {
        assert!(read_num(b"abc").is_err());
        assert!(read_num(b"+abc").is_err());
        assert!(read_num(b"1a").is_err());
    }
}
