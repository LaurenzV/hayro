//! Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/calculate_sha256.js>.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
    0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[inline]
const fn rotr(x: u32, n: u32) -> u32 {
    (x >> n) | (x << (32 - n))
}

#[inline]
const fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

#[inline]
const fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

#[inline]
const fn sigma_0(x: u32) -> u32 {
    rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22)
}

#[inline]
const fn sigma_1(x: u32) -> u32 {
    rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25)
}

#[inline]
const fn little_sigma_0(x: u32) -> u32 {
    rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3)
}

#[inline]
const fn little_sigma_1(x: u32) -> u32 {
    rotr(x, 17) ^ rotr(x, 19) ^ (x >> 10)
}

pub(crate) fn calculate(data: &[u8]) -> [u8; 32] {
    // Initial hash values (first 32 bits of the fractional parts of the square roots of the first 8 primes)
    let mut h = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];

    // Pre-processing: padding
    let bit_len = data.len() as u64 * 8;
    let padded_len = ((data.len() + 9 + 63) / 64) * 64; // Round up to nearest multiple of 64
    let mut padded = vec![0u8; padded_len];

    // Copy original data
    padded[..data.len()].copy_from_slice(data);

    // Add padding bit
    padded[data.len()] = 0x80;

    // Add length as 64-bit big-endian integer at the end
    let len_bytes = bit_len.to_be_bytes();
    padded[padded_len - 8..].copy_from_slice(&len_bytes);

    // Process message in 512-bit (64-byte) chunks
    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];

        // Break chunk into sixteen 32-bit big-endian words
        for (i, word_bytes) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([
                word_bytes[0],
                word_bytes[1],
                word_bytes[2],
                word_bytes[3],
            ]);
        }

        // Extend the first 16 words into the remaining 48 words
        for i in 16..64 {
            w[i] = little_sigma_1(w[i - 2])
                .wrapping_add(w[i - 7])
                .wrapping_add(little_sigma_0(w[i - 15]))
                .wrapping_add(w[i - 16]);
        }

        // Initialize working variables
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h_var] = h;

        // Main loop
        for i in 0..64 {
            let temp1 = h_var
                .wrapping_add(sigma_1(e))
                .wrapping_add(ch(e, f, g))
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let temp2 = sigma_0(a).wrapping_add(maj(a, b, c));

            h_var = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        // Add this chunk's hash to result so far
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(h_var);
    }

    // Produce the final hash value as a 256-bit number (32 bytes)
    let mut result = [0u8; 32];
    for (i, &hash_word) in h.iter().enumerate() {
        let bytes = hash_word.to_be_bytes();
        result[i * 4..(i + 1) * 4].copy_from_slice(&bytes);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correctness() {
        assert_eq!(
            calculate(b""),
            [0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
                0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55]
        );

        assert_eq!(
            calculate(b"a"),
            [0xca, 0x97, 0x81, 0x12, 0xca, 0x1b, 0xbd, 0xca, 0xfa, 0xc2, 0x31, 0xb3, 0x9a, 0x23, 0xdc, 0x4d,
                0xa7, 0x86, 0xef, 0xf8, 0x14, 0x7c, 0x4e, 0x72, 0xb9, 0x80, 0x77, 0x85, 0xaf, 0xee, 0x48, 0xbb]
        );

        assert_eq!(
            calculate(b"abc"),
            [0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
                0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad]
        );

        assert_eq!(
            calculate(b"Hello, World!"),
            [0xdf, 0xfd, 0x60, 0x21, 0xbb, 0x2b, 0xd5, 0xb0, 0xaf, 0x67, 0x62, 0x90, 0x80, 0x9e, 0xc3, 0xa5,
                0x31, 0x91, 0xdd, 0x81, 0xc7, 0xf7, 0x0a, 0x4b, 0x28, 0x68, 0x8a, 0x36, 0x21, 0x82, 0x98, 0x6f]
        );

        assert_eq!(
            calculate(&[0xde, 0xad, 0xbe, 0xef]),
            [0x5f, 0x78, 0xc3, 0x32, 0x74, 0xe4, 0x3f, 0xa9, 0xde, 0x56, 0x59, 0x26, 0x5c, 0x1d, 0x91, 0x7e,
                0x25, 0xc0, 0x37, 0x22, 0xdc, 0xb0, 0xb8, 0xd2, 0x7d, 0xb8, 0xd5, 0xfe, 0xaa, 0x81, 0x39, 0x53]
        );
    }
}