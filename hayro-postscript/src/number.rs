use crate::reader::{Reader, is_delimiter, is_whitespace};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Number {
    Integer(i32),
    Real(f64),
}

fn is_terminated(r: &Reader<'_>) -> bool {
    match r.peek_byte() {
        None => true,
        Some(b) => is_whitespace(b) || is_delimiter(b),
    }
}

pub(crate) fn read(r: &mut Reader<'_>) -> Option<Number> {
    let saved = r.offset();

    // Optional sign.
    let first = r.peek_byte()?;
    let has_sign = first == b'+' || first == b'-';
    if has_sign {
        r.forward();
    }

    // Consume leading digits.
    let digit_start = r.offset();
    r.forward_while(|b| b.is_ascii_digit());
    let has_digits = r.offset() > digit_start;

    // Check for radix syntax: `base#digits` (no sign allowed).
    if !has_sign && has_digits && r.peek_byte() == Some(b'#') {
        let base_bytes = r.range(digit_start..r.offset())?;
        let base_str = core::str::from_utf8(base_bytes).ok()?;
        let base = base_str.parse::<u32>().ok()?;

        if !(2..=36).contains(&base) {
            r.jump(saved);
            return None;
        }

        r.forward(); // skip '#'

        let num_start = r.offset();
        r.forward_while(|b| b.is_ascii_alphanumeric());

        if r.offset() == num_start {
            r.jump(saved);
            return None;
        }

        if !is_terminated(r) {
            r.jump(saved);
            return None;
        }

        let num_bytes = r.range(num_start..r.offset())?;
        let num_str = core::str::from_utf8(num_bytes).ok()?;
        let value = i32::from_str_radix(num_str, base).ok()?;

        return Some(Number::Integer(value));
    }

    // Check for real number indicators: `.` or `e`/`E`.
    let has_dot = r.peek_byte() == Some(b'.');
    if has_dot {
        r.forward(); // skip '.'
        r.forward_while(|b| b.is_ascii_digit());
    }

    // At this point we need at least some digits (before or after the dot).
    if !has_digits && !has_dot {
        r.jump(saved);
        return None;
    }

    // For the pure-integer path (no dot, no exponent), check early.
    let has_exponent = matches!(r.peek_byte(), Some(b'e' | b'E'));
    if has_exponent {
        r.forward(); // skip 'e'/'E'
        // Optional exponent sign.
        if matches!(r.peek_byte(), Some(b'+' | b'-')) {
            r.forward();
        }
        r.forward_while(|b| b.is_ascii_digit());
    }

    if !is_terminated(r) {
        r.jump(saved);
        return None;
    }

    let token = r.range(saved..r.offset())?;
    let s = core::str::from_utf8(token).ok()?;

    if has_dot || has_exponent {
        let value = s.parse::<f64>().ok()?;
        Some(Number::Real(value))
    } else {
        // Must have had leading digits for a plain integer.
        if !has_digits {
            r.jump(saved);
            return None;
        }
        let value = s.parse::<i32>().ok()?;
        Some(Number::Integer(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_num(input: &[u8]) -> Option<Number> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    #[test]
    fn signed_integers() {
        // 123 -98 43445 0 +17
        assert_eq!(read_num(b"123 "), Some(Number::Integer(123)));
        assert_eq!(read_num(b"-98 "), Some(Number::Integer(-98)));
        assert_eq!(read_num(b"43445 "), Some(Number::Integer(43445)));
        assert_eq!(read_num(b"0 "), Some(Number::Integer(0)));
        assert_eq!(read_num(b"+17 "), Some(Number::Integer(17)));
    }

    #[test]
    fn real_numbers() {
        // -.002 34.5 -3.62 123.6e10 1.0E-5 1E6 -1. 0.0
        assert_eq!(read_num(b"-.002 "), Some(Number::Real(-0.002)));
        assert_eq!(read_num(b"34.5 "), Some(Number::Real(34.5)));
        assert_eq!(read_num(b"-3.62 "), Some(Number::Real(-3.62)));
        assert_eq!(read_num(b"123.6e10 "), Some(Number::Real(123.6e10)));
        assert_eq!(read_num(b"1.0E-5 "), Some(Number::Real(1.0E-5)));
        assert_eq!(read_num(b"1E6 "), Some(Number::Real(1E6)));
        assert_eq!(read_num(b"-1. "), Some(Number::Real(-1.0)));
        assert_eq!(read_num(b"0.0 "), Some(Number::Real(0.0)));
    }

    #[test]
    fn radix_numbers() {
        // 8#1777 16#FFFE 2#1000
        assert_eq!(read_num(b"8#1777 "), Some(Number::Integer(0o1777)));
        assert_eq!(read_num(b"16#FFFE "), Some(Number::Integer(0xFFFE)));
        assert_eq!(read_num(b"2#1000 "), Some(Number::Integer(0b1000)));
    }

    #[test]
    fn not_a_number() {
        assert_eq!(read_num(b"abc"), None);
    }

    #[test]
    fn sign_only() {
        assert_eq!(read_num(b"+abc"), None);
    }

    #[test]
    fn mixed_alphanum_fallthrough() {
        assert_eq!(read_num(b"1a"), None);
    }
}
