//! Integer parsing.

use crate::reader::{Reader, is_delimiter, is_whitespace};

/// Try to read an integer from the current position.
///
/// Returns `Some(i32)` if the token starting at the current position is a valid
/// integer (optional sign followed by digits, terminated by whitespace, delimiter
/// or EOF). Returns `None` (without advancing the reader) if it isn't.
pub(crate) fn read(r: &mut Reader<'_>) -> Option<i32> {
    let saved = r.offset();

    // Optional sign.
    let first = r.peek_byte()?;
    if first == b'+' || first == b'-' {
        r.forward();
    }

    // Must have at least one digit.
    let digit_start = r.offset();
    r.forward_while(|b| b.is_ascii_digit());

    if r.offset() == digit_start {
        r.jump(saved);
        return None;
    }

    // Must be followed by whitespace, delimiter, or EOF.
    if let Some(next) = r.peek_byte() {
        if !is_whitespace(next) && !is_delimiter(next) {
            r.jump(saved);
            return None;
        }
    }

    let token = r.range(saved..r.offset())?;
    let s = core::str::from_utf8(token).ok()?;
    let value = s.parse::<i32>().ok()?;

    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_int(input: &[u8]) -> Option<i32> {
        let mut r = Reader::new(input);
        read(&mut r)
    }

    #[test]
    fn positive() {
        assert_eq!(read_int(b"42 "), Some(42));
    }

    #[test]
    fn negative() {
        assert_eq!(read_int(b"-7 "), Some(-7));
    }

    #[test]
    fn with_plus() {
        assert_eq!(read_int(b"+3 "), Some(3));
    }

    #[test]
    fn zero() {
        assert_eq!(read_int(b"0 "), Some(0));
    }

    #[test]
    fn leading_zeros() {
        assert_eq!(read_int(b"007 "), Some(7));
    }

    #[test]
    fn at_eof() {
        assert_eq!(read_int(b"123"), Some(123));
    }

    #[test]
    fn before_delimiter() {
        assert_eq!(read_int(b"5("), Some(5));
    }

    #[test]
    fn not_a_number() {
        assert_eq!(read_int(b"abc"), None);
    }

    #[test]
    fn mixed_alphanum_fallthrough() {
        // `1a` should not parse as integer (not terminated by ws/delim).
        assert_eq!(read_int(b"1a"), None);
    }

    #[test]
    fn sign_only() {
        assert_eq!(read_int(b"+abc"), None);
    }
}
