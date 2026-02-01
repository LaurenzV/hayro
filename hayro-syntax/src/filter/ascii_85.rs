use alloc::vec::Vec;

/// Decode ASCII base-85 encoded data (PDF spec 7.4.3).
///
/// ## Algorithm derivation
///
/// ASCII85 encodes 4 binary bytes into 5 printable ASCII characters.
///
/// **Encoding (to understand decoding):**
/// - Take 4 bytes as a big-endian u32: `value = b1×256³ + b2×256² + b3×256 + b4`
/// - Convert to base-85: repeatedly divide by 85, taking remainders
/// - Add 33 to each digit to get ASCII characters in range `!` (33) to `u` (117)
/// - Special case: if all 4 bytes are zero, emit `z` instead of `!!!!!`
///
/// **Decoding (what we implement):**
/// - Take 5 ASCII characters, subtract 33 from each to get base-85 digits (0-84)
/// - Reconstruct the u32: `value = d1×85⁴ + d2×85³ + d3×85² + d4×85 + d5`
/// - Split into 4 bytes (big-endian)
/// - Special case: `z` decodes to 4 zero bytes
///
/// **Partial groups:**
/// - If final group has n characters (2 ≤ n ≤ 4), pad with `u` (84) to make 5
/// - Decode normally, but only output (n-1) bytes
///
/// **Powers of 85:** 85⁰=1, 85¹=85, 85²=7225, 85³=614125, 85⁴=52200625
pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
    const POW85: [u32; 5] = [52200625, 614125, 7225, 85, 1];

    // Strip EOD marker if present, ignore everything after it.
    let data = match data.windows(2).position(|w| w == b"~>") {
        Some(pos) => &data[..pos],
        None => data,
    };

    // Filter whitespace and collect valid characters.
    let mut chars: Vec<u8> = Vec::with_capacity(data.len());
    for &b in data {
        match b {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0C => {} // Skip whitespace
            b'!'..=b'u' | b'z' => chars.push(b),
            _ => return None, // Invalid character
        }
    }

    let mut decoded = Vec::with_capacity(chars.len() * 4 / 5);
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == b'z' {
            // Special case: 'z' represents four zero bytes.
            decoded.extend_from_slice(&[0, 0, 0, 0]);
            i += 1;
        } else {
            // Determine group size (5 for full, 2-4 for final partial).
            let remaining = chars.len() - i;
            let group_len = remaining.min(5);

            if group_len < 2 {
                // Single character at end is invalid per spec.
                return None;
            }

            // Compute value from base-85 digits, padding partial groups with 'u' (84).
            let mut value: u32 = 0;
            for j in 0..5 {
                let digit = if j < group_len {
                    (chars[i + j] - b'!') as u32
                } else {
                    84 // Pad with 'u'
                };
                value = value.checked_add(digit.checked_mul(POW85[j])?)?;
            }

            // Check for overflow (max valid is 2^32 - 1).
            // A 5-character group could theoretically encode > 2^32.

            // Extract bytes (big-endian).
            let bytes = value.to_be_bytes();

            // Output (group_len - 1) bytes for partial groups, 4 for full.
            let output_len = if group_len == 5 { 4 } else { group_len - 1 };
            decoded.extend_from_slice(&bytes[..output_len]);

            i += group_len;
        }
    }

    Some(decoded)
}

#[cfg(test)]
mod tests {
    use crate::filter::ascii_85::decode;

    #[test]
    fn decode_simple() {
        let input = b"87cURDZ~>";
        assert_eq!(decode(input).unwrap(), b"Hello");
    }

    #[test]
    fn decode_spaces() {
        let input = b"87  cURD  Z~>";
        assert_eq!(decode(input).unwrap(), b"Hello");
    }

    #[test]
    fn decode_zeroes() {
        let input = b"z~>";
        assert_eq!(decode(input).unwrap(), [0, 0, 0, 0]);
    }
}
