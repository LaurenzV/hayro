const SHIFT_AMOUNTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const CONSTANTS: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

pub(crate) fn calculate(data: &[u8]) -> [u8; 16] {
    let mut state = [0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32];

    let original_len = data.len();
    let mut message = data.to_vec();
    message.push(0x80);

    while (message.len() % 64) != 56 {
        message.push(0);
    }

    message.extend_from_slice(&(original_len as u64 * 8).to_le_bytes());

    for chunk in message.chunks_exact(64) {
        let words: Vec<u32> = chunk
            .chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();

        let mut working_vars = state;

        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => (
                    (working_vars[1] & working_vars[2]) | (!working_vars[1] & working_vars[3]),
                    i,
                ),
                16..=31 => (
                    (working_vars[3] & working_vars[1]) | (!working_vars[3] & working_vars[2]),
                    (5 * i + 1) % 16,
                ),
                32..=47 => (
                    working_vars[1] ^ working_vars[2] ^ working_vars[3],
                    (3 * i + 5) % 16,
                ),
                48..=63 => (
                    working_vars[2] ^ (working_vars[1] | !working_vars[3]),
                    (7 * i) % 16,
                ),
                _ => unreachable!(),
            };

            let temp = working_vars[3];
            working_vars[3] = working_vars[2];
            working_vars[2] = working_vars[1];
            working_vars[1] = working_vars[1].wrapping_add(
                (working_vars[0]
                    .wrapping_add(f)
                    .wrapping_add(CONSTANTS[i])
                    .wrapping_add(words[g]))
                .rotate_left(SHIFT_AMOUNTS[i]),
            );
            working_vars[0] = temp;
        }

        for (state_val, working_val) in state.iter_mut().zip(working_vars.iter()) {
            *state_val = state_val.wrapping_add(*working_val);
        }
    }

    let mut result = [0u8; 16];
    for (i, &word) in state.iter().enumerate() {
        result[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests were cross-compared against the md5 crate.

    #[test]
    fn test_empty_and_short_inputs() {
        assert_eq!(
            calculate(b""),
            [
                0xd4, 0x1d, 0x8c, 0xd9, 0x8f, 0x00, 0xb2, 0x04, 0xe9, 0x80, 0x09, 0x98, 0xec, 0xf8,
                0x42, 0x7e
            ]
        );
        assert_eq!(
            calculate(b"a"),
            [
                0x0c, 0xc1, 0x75, 0xb9, 0xc0, 0xf1, 0xb6, 0xa8, 0x31, 0xc3, 0x99, 0xe2, 0x69, 0x77,
                0x26, 0x61
            ]
        );
        assert_eq!(
            calculate(b"abc"),
            [
                0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1,
                0x7f, 0x72
            ]
        );
    }

    #[test]
    fn test_common_text() {
        assert_eq!(
            calculate(b"Hello, World!"),
            [
                0x65, 0xa8, 0xe2, 0x7d, 0x88, 0x79, 0x28, 0x38, 0x31, 0xb6, 0x64, 0xbd, 0x8b, 0x7f,
                0x0a, 0xd4
            ]
        );
        assert_eq!(
            calculate(b"The quick brown fox jumps over the lazy dog"),
            [
                0x9e, 0x10, 0x7d, 0x9d, 0x37, 0x2b, 0xb6, 0x82, 0x6b, 0xd8, 0x1d, 0x35, 0x42, 0xa4,
                0x19, 0xd6
            ]
        );
        assert_eq!(
            calculate(b"abcdefghijklmnopqrstuvwxyz"),
            [
                0xc3, 0xfc, 0xd3, 0xd7, 0x61, 0x92, 0xe4, 0x00, 0x7d, 0xfb, 0x49, 0x6c, 0xca, 0x67,
                0xe1, 0x3b
            ]
        );
    }

    #[test]
    fn test_block_boundaries() {
        assert_eq!(
            calculate(b"1234567890123456789012345678901234567890123456789012345"),
            [
                0xc9, 0xcc, 0xf1, 0x68, 0x91, 0x4a, 0x1b, 0xcf, 0xc3, 0x22, 0x9f, 0x19, 0x48, 0xe6,
                0x7d, 0xa0
            ]
        );
        assert_eq!(
            calculate(b"1234567890123456789012345678901234567890123456789012345678901234"),
            [
                0xeb, 0x6c, 0x41, 0x79, 0xc0, 0xa7, 0xc8, 0x2c, 0xc2, 0x82, 0x8c, 0x1e, 0x63, 0x38,
                0xe1, 0x65
            ]
        );
    }

    #[test]
    fn test_binary_and_special_data() {
        assert_eq!(
            calculate(&[0u8; 100]),
            [
                0x6d, 0x0b, 0xb0, 0x09, 0x54, 0xce, 0xb7, 0xfb, 0xee, 0x43, 0x6b, 0xb5, 0x5a, 0x83,
                0x97, 0xa9
            ]
        );
        assert_eq!(
            calculate(&(0u8..=255u8).collect::<Vec<u8>>()),
            [
                0xe2, 0xc8, 0x65, 0xdb, 0x41, 0x62, 0xbe, 0xd9, 0x63, 0xbf, 0xaa, 0x9e, 0xf6, 0xac,
                0x18, 0xf0
            ]
        );
        assert_eq!(
            calculate("🦀💎".as_bytes()),
            [
                0xbc, 0xab, 0xb9, 0x04, 0x18, 0xab, 0x44, 0xce, 0xca, 0xa9, 0x53, 0xd9, 0x73, 0x9f,
                0x85, 0x0c
            ]
        );
    }

    #[test]
    fn test_large_and_repetitive() {
        assert_eq!(
            calculate(&b"0123456789".repeat(100)),
            [
                0x42, 0x70, 0x08, 0xb3, 0xfe, 0x19, 0x2f, 0x66, 0x3d, 0x66, 0x5f, 0x56, 0xcd, 0x75,
                0x71, 0x6c
            ]
        );
        assert_eq!(
            calculate(b"1234567890"),
            [
                0xe8, 0x07, 0xf1, 0xfc, 0xf8, 0x2d, 0x13, 0x2f, 0x9b, 0xb0, 0x18, 0xca, 0x67, 0x38,
                0xa1, 0x9f
            ]
        );
        assert_eq!(
            calculate(b"abcabcabc"),
            [
                0x97, 0xac, 0x82, 0xa5, 0xb8, 0x25, 0x23, 0x9e, 0x78, 0x2d, 0x03, 0x39, 0xe2, 0xd7,
                0xb9, 0x10
            ]
        );
    }
}
