//! Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/crypto.js>.

const S_BOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const INV_S_BOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

struct AESCore;

impl AESCore {
    fn sub_bytes(state: &mut [u8; 16]) {
        state
            .iter_mut()
            .for_each(|byte| *byte = S_BOX[*byte as usize]);
    }

    fn inv_sub_bytes(state: &mut [u8; 16]) {
        state
            .iter_mut()
            .for_each(|byte| *byte = INV_S_BOX[*byte as usize]);
    }

    fn shift_rows(state: &mut [u8; 16]) {
        let temp = state[1];
        state[1] = state[5];
        state[5] = state[9];
        state[9] = state[13];
        state[13] = temp;

        let temp1 = state[2];
        let temp2 = state[6];
        state[2] = state[10];
        state[6] = state[14];
        state[10] = temp1;
        state[14] = temp2;

        let temp = state[15];
        state[15] = state[11];
        state[11] = state[7];
        state[7] = state[3];
        state[3] = temp;
    }

    fn inv_shift_rows(state: &mut [u8; 16]) {
        let temp = state[13];
        state[13] = state[9];
        state[9] = state[5];
        state[5] = state[1];
        state[1] = temp;

        let temp1 = state[2];
        let temp2 = state[6];
        state[2] = state[10];
        state[6] = state[14];
        state[10] = temp1;
        state[14] = temp2;

        let temp = state[3];
        state[3] = state[7];
        state[7] = state[11];
        state[11] = state[15];
        state[15] = temp;
    }

    fn gf_mul(a: u8, b: u8) -> u8 {
        let mut result = 0;
        let mut aa = a;
        let mut bb = b;

        for _ in 0..8 {
            if bb & 1 != 0 {
                result ^= aa;
            }
            let carry = aa & 0x80 != 0;
            aa <<= 1;
            if carry {
                aa ^= 0x1b;
            }
            bb >>= 1;
        }
        result
    }

    fn mix_columns(state: &mut [u8; 16]) {
        (0..4).for_each(|i| {
            let col = i * 4;
            let [s0, s1, s2, s3] = [state[col], state[col + 1], state[col + 2], state[col + 3]];

            state[col] = Self::gf_mul(0x02, s0) ^ Self::gf_mul(0x03, s1) ^ s2 ^ s3;
            state[col + 1] = s0 ^ Self::gf_mul(0x02, s1) ^ Self::gf_mul(0x03, s2) ^ s3;
            state[col + 2] = s0 ^ s1 ^ Self::gf_mul(0x02, s2) ^ Self::gf_mul(0x03, s3);
            state[col + 3] = Self::gf_mul(0x03, s0) ^ s1 ^ s2 ^ Self::gf_mul(0x02, s3);
        });
    }

    fn inv_mix_columns(state: &mut [u8; 16]) {
        (0..4).for_each(|i| {
            let col = i * 4;
            let [s0, s1, s2, s3] = [state[col], state[col + 1], state[col + 2], state[col + 3]];

            state[col] = Self::gf_mul(0x0e, s0)
                ^ Self::gf_mul(0x0b, s1)
                ^ Self::gf_mul(0x0d, s2)
                ^ Self::gf_mul(0x09, s3);
            state[col + 1] = Self::gf_mul(0x09, s0)
                ^ Self::gf_mul(0x0e, s1)
                ^ Self::gf_mul(0x0b, s2)
                ^ Self::gf_mul(0x0d, s3);
            state[col + 2] = Self::gf_mul(0x0d, s0)
                ^ Self::gf_mul(0x09, s1)
                ^ Self::gf_mul(0x0e, s2)
                ^ Self::gf_mul(0x0b, s3);
            state[col + 3] = Self::gf_mul(0x0b, s0)
                ^ Self::gf_mul(0x0d, s1)
                ^ Self::gf_mul(0x09, s2)
                ^ Self::gf_mul(0x0e, s3);
        });
    }

    fn add_round_key(state: &mut [u8; 16], round_key: &[u8; 16]) {
        state
            .iter_mut()
            .zip(round_key.iter())
            .for_each(|(s, &k)| *s ^= k);
    }
}

#[derive(Clone)]
pub(crate) struct AESCipher<const KEY_SIZE: usize, const ROUNDS: usize> {
    round_keys: [[u8; 16]; ROUNDS],
}

pub(crate) type AES128Cipher = AESCipher<16, 11>;
pub(crate) type AES256Cipher = AESCipher<32, 15>;

impl<const KEY_SIZE: usize, const ROUNDS: usize> AESCipher<KEY_SIZE, ROUNDS> {
    pub(crate) fn new(key: &[u8]) -> Option<Self> {
        if key.len() != KEY_SIZE {
            return None;
        }

        let mut round_keys = [[0u8; 16]; ROUNDS];

        match KEY_SIZE {
            16 => Self::expand_key_128(&mut round_keys, key),
            32 => Self::expand_key_256(&mut round_keys, key),
            _ => return None,
        }

        Some(AESCipher { round_keys })
    }

    fn expand_key_128(round_keys: &mut [[u8; 16]; ROUNDS], key: &[u8]) {
        round_keys[0].copy_from_slice(&key[..16]);

        (1..ROUNDS).for_each(|i| {
            let mut temp = [0u8; 4];
            temp.copy_from_slice(&round_keys[i - 1][12..16]);

            temp.rotate_left(1);
            temp.iter_mut().for_each(|b| *b = S_BOX[*b as usize]);

            temp[0] ^= RCON[i - 1];

            (0..4).for_each(|j| {
                (0..4).for_each(|k| {
                    round_keys[i][j * 4 + k] = round_keys[i - 1][j * 4 + k] ^ temp[k];
                });
                if j < 3 {
                    temp.copy_from_slice(&round_keys[i][j * 4..(j + 1) * 4]);
                }
            });
        });
    }

    fn expand_key_256(round_keys: &mut [[u8; 16]; ROUNDS], key: &[u8]) {
        round_keys[0].copy_from_slice(&key[0..16]);
        round_keys[1].copy_from_slice(&key[16..32]);

        (2..ROUNDS).for_each(|i| {
            let mut temp = [0u8; 4];

            if i % 2 == 0 {
                temp.copy_from_slice(&round_keys[i - 1][12..16]);
                temp.rotate_left(1);
                temp.iter_mut().for_each(|b| *b = S_BOX[*b as usize]);
                temp[0] ^= RCON[(i / 2) - 1];
            } else {
                temp.copy_from_slice(&round_keys[i - 1][12..16]);
                temp.iter_mut().for_each(|b| *b = S_BOX[*b as usize]);
            }

            (0..4).for_each(|j| {
                (0..4).for_each(|k| {
                    round_keys[i][j * 4 + k] = round_keys[i - 2][j * 4 + k] ^ temp[k];
                });
                if j < 3 {
                    temp.copy_from_slice(&round_keys[i][j * 4..(j + 1) * 4]);
                }
            });
        });
    }

    pub(crate) fn encrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let mut state = *input;

        AESCore::add_round_key(&mut state, &self.round_keys[0]);

        let main_rounds = ROUNDS - 1;
        (1..main_rounds).for_each(|round| {
            AESCore::sub_bytes(&mut state);
            AESCore::shift_rows(&mut state);
            AESCore::mix_columns(&mut state);
            AESCore::add_round_key(&mut state, &self.round_keys[round]);
        });

        AESCore::sub_bytes(&mut state);
        AESCore::shift_rows(&mut state);
        AESCore::add_round_key(&mut state, &self.round_keys[main_rounds]);

        state
    }

    pub(crate) fn decrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let mut state = *input;

        let main_rounds = ROUNDS - 1;

        AESCore::add_round_key(&mut state, &self.round_keys[main_rounds]);
        AESCore::inv_shift_rows(&mut state);
        AESCore::inv_sub_bytes(&mut state);

        (1..main_rounds).rev().for_each(|round| {
            AESCore::add_round_key(&mut state, &self.round_keys[round]);
            AESCore::inv_mix_columns(&mut state);
            AESCore::inv_shift_rows(&mut state);
            AESCore::inv_sub_bytes(&mut state);
        });

        AESCore::add_round_key(&mut state, &self.round_keys[0]);

        state
    }

    pub(crate) fn encrypt_cbc(&self, data: &[u8], iv: &[u8; 16]) -> Vec<u8> {
        let mut result = Vec::new();
        let mut current_iv = *iv;

        let mut padded_data = data.to_vec();
        let pad_len = 16 - (data.len() % 16);
        if pad_len == 0 {
            padded_data.extend(vec![16u8; 16]);
        } else {
            padded_data.extend(vec![pad_len as u8; pad_len]);
        }

        for chunk in padded_data.chunks(16) {
            let mut block = [0u8; 16];
            block.copy_from_slice(chunk);

            for i in 0..16 {
                block[i] ^= current_iv[i];
            }

            let encrypted = self.encrypt_block(&block);
            result.extend_from_slice(&encrypted);
            current_iv = encrypted;
        }

        result
    }

    pub(crate) fn decrypt_cbc(&self, data: &[u8], iv: &[u8; 16], unpad: bool) -> Vec<u8> {
        let mut result = Vec::new();
        let mut prev_block = *iv;

        for chunk in data.chunks_exact(16) {
            let mut block = [0u8; 16];
            block.copy_from_slice(chunk);

            let decrypted = self.decrypt_block(&block);

            let mut plain_block = [0u8; 16];
            for i in 0..16 {
                plain_block[i] = decrypted[i] ^ prev_block[i];
            }

            result.extend_from_slice(&plain_block);
            prev_block = block;
        }

        if unpad
            && let Some(&pad_len) = result.last()
            && pad_len > 0
            && pad_len <= 16
            && result.len() >= pad_len as usize
        {
            let start = result.len() - pad_len as usize;
            if result[start..].iter().all(|&b| b == pad_len) {
                result.truncate(start);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::{
        BlockDecryptMut, BlockEncrypt, BlockEncryptMut, KeyInit, generic_array::GenericArray,
    };
    use aes::{Aes128, Aes256};
    use cbc::{
        Decryptor, Encryptor,
        cipher::{KeyIvInit, block_padding::Pkcs7},
    };

    type Aes128Cbc = Encryptor<Aes128>;
    type Aes128CbcDec = Decryptor<Aes128>;
    type Aes256Cbc = Encryptor<Aes256>;
    type Aes256CbcDec = Decryptor<Aes256>;

    fn aes_128_block_test(key: &[u8; 16], plaintext: &[u8; 16]) {
        let our_cipher = AES128Cipher::new(key).unwrap();

        let external_cipher = Aes128::new_from_slice(key).unwrap();
        let mut external_block = GenericArray::clone_from_slice(plaintext);
        external_cipher.encrypt_block(&mut external_block);

        let our_ciphertext = our_cipher.encrypt_block(plaintext);
        assert_eq!(
            our_ciphertext,
            external_block.as_slice(),
            "encryption should match"
        );

        let our_decrypted = our_cipher.decrypt_block(&our_ciphertext);
        assert_eq!(
            our_decrypted, *plaintext,
            "roundtrip should recover original plaintext"
        );
    }

    fn aes_256_block_test(key: &[u8; 32], plaintext: &[u8; 16]) {
        let our_cipher = AES256Cipher::new(key).unwrap();

        let external_cipher = Aes256::new_from_slice(key).unwrap();
        let mut external_block = GenericArray::clone_from_slice(plaintext);
        external_cipher.encrypt_block(&mut external_block);

        let our_ciphertext = our_cipher.encrypt_block(plaintext);
        assert_eq!(
            our_ciphertext,
            external_block.as_slice(),
            "encryption should match"
        );

        let our_decrypted = our_cipher.decrypt_block(&our_ciphertext);
        assert_eq!(
            our_decrypted, *plaintext,
            "roundtrip should recover original plaintext"
        );
    }

    fn aes_128_cbc_test(key: &[u8; 16], iv: &[u8; 16], plaintext: &[u8]) {
        let our_cipher = AES128Cipher::new(key).unwrap();

        let our_ciphertext = our_cipher.encrypt_cbc(plaintext, iv);

        let external_decryptor = Aes128CbcDec::new_from_slices(key, iv).unwrap();
        let mut external_buffer = our_ciphertext.clone();
        let external_decrypted = external_decryptor
            .decrypt_padded_mut::<Pkcs7>(&mut external_buffer)
            .unwrap();

        assert_eq!(
            external_decrypted, plaintext,
            "AES-128 CBC: our encryption should be decryptable by external crate"
        );

        let external_encryptor = Aes128Cbc::new_from_slices(key, iv).unwrap();
        let mut buffer = plaintext.to_vec();
        buffer.resize(plaintext.len() + 16, 0);
        let external_ciphertext = external_encryptor
            .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
            .unwrap();

        let our_decrypted = our_cipher.decrypt_cbc(external_ciphertext, iv, true);
        assert_eq!(
            our_decrypted, plaintext,
            "AES-128 CBC: external encryption should be decryptable by our implementation"
        );

        let our_roundtrip = our_cipher.decrypt_cbc(&our_ciphertext, iv, true);
        assert_eq!(
            our_roundtrip, plaintext,
            "AES-128 CBC: our roundtrip should work"
        );
    }

    fn aes_256_cbc_test(key: &[u8; 32], iv: &[u8; 16], plaintext: &[u8]) {
        let our_cipher = AES256Cipher::new(key).unwrap();

        let our_ciphertext = our_cipher.encrypt_cbc(plaintext, iv);

        let external_decryptor = Aes256CbcDec::new_from_slices(key, iv).unwrap();
        let mut external_buffer = our_ciphertext.clone();
        let external_decrypted = external_decryptor
            .decrypt_padded_mut::<Pkcs7>(&mut external_buffer)
            .unwrap();

        assert_eq!(
            external_decrypted, plaintext,
            "AES-256 CBC: our encryption should be decryptable by external crate"
        );

        let external_encryptor = Aes256Cbc::new_from_slices(key, iv).unwrap();
        let mut buffer = plaintext.to_vec();
        buffer.resize(plaintext.len() + 16, 0);
        let external_ciphertext = external_encryptor
            .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
            .unwrap();

        let our_decrypted = our_cipher.decrypt_cbc(external_ciphertext, iv, true);
        assert_eq!(
            our_decrypted, plaintext,
            "AES-256 CBC: external encryption should be decryptable by our implementation"
        );

        let our_roundtrip = our_cipher.decrypt_cbc(&our_ciphertext, iv, true);
        assert_eq!(
            our_roundtrip, plaintext,
            "AES-256 CBC: our roundtrip should work"
        );
    }

    #[test]
    fn test_aes128_block_operations() {
        let test_cases = [
            (
                [
                    0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09,
                    0xcf, 0x4f, 0x3c,
                ],
                [
                    0x32, 0x43, 0xf6, 0xa8, 0x88, 0x5a, 0x30, 0x8d, 0x31, 0x31, 0x98, 0xa2, 0xe0,
                    0x37, 0x07, 0x34,
                ],
            ),
            (
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f,
                ],
                [
                    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
                    0xdd, 0xee, 0xff,
                ],
            ),
            ([0xff; 16], [0x00; 16]),
            ([0x00; 16], [0xff; 16]),
        ];

        for (key, plaintext) in test_cases {
            aes_128_block_test(&key, &plaintext);
        }
    }

    #[test]
    fn test_aes256_block_operations() {
        let test_cases = [
            (
                [
                    0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe, 0x2b, 0x73, 0xae, 0xf0, 0x85,
                    0x7d, 0x77, 0x81, 0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7, 0x2d, 0x98,
                    0x10, 0xa3, 0x09, 0x14, 0xdf, 0xf4,
                ],
                [
                    0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73,
                    0x93, 0x17, 0x2a,
                ],
            ),
            (
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
                    0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
                ],
                [
                    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
                    0xdd, 0xee, 0xff,
                ],
            ),
            ([0xff; 32], [0x00; 16]),
            ([0x00; 32], [0xff; 16]),
        ];

        for (key, plaintext) in test_cases {
            aes_256_block_test(&key, &plaintext);
        }
    }

    #[test]
    fn test_aes128_encryption_decryption() {
        let test_cases = [
            (
                [
                    0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09,
                    0xcf, 0x4f, 0x3c,
                ],
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f,
                ],
                b"Hello, World!" as &[u8],
            ),
            (
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f,
                ],
                [0xff; 16],
                b"The quick brown fox jumps over the lazy dog",
            ),
            (
                [0xff; 16],
                [0x00; 16],
                b"AES-128 encryption test vector for CBC mode with PKCS7 padding",
            ),
            ([0xaa; 16], [0x55; 16], &[0x42u8; 100]),
        ];

        for (key, iv, plaintext) in test_cases {
            aes_128_cbc_test(&key, &iv, plaintext);
        }
    }

    #[test]
    fn test_aes256_encryption_decryption() {
        let test_cases = [
            (
                [
                    0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe, 0x2b, 0x73, 0xae, 0xf0, 0x85,
                    0x7d, 0x77, 0x81, 0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7, 0x2d, 0x98,
                    0x10, 0xa3, 0x09, 0x14, 0xdf, 0xf4,
                ],
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f,
                ],
                b"Hello, World!" as &[u8],
            ),
            (
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
                    0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
                ],
                [0xff; 16],
                b"The quick brown fox jumps over the lazy dog",
            ),
            (
                [0xff; 32],
                [0x00; 16],
                b"AES-256 encryption test vector for CBC mode with PKCS7 padding",
            ),
            ([0xaa; 32], [0x55; 16], &[0x42u8; 100]),
        ];

        for (key, iv, plaintext) in test_cases {
            aes_256_cbc_test(&key, &iv, plaintext);
        }
    }

    #[test]
    fn test_aes128_cbc_decryption() {
        let test_cases = [
            (
                [
                    0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09,
                    0xcf, 0x4f, 0x3c,
                ],
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f,
                ],
                b"Hello, World!" as &[u8],
            ),
            (
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f,
                ],
                [0xff; 16],
                b"The quick brown fox jumps over the lazy dog",
            ),
            (
                [0xff; 16],
                [0x00; 16],
                b"AES-128 test vector for CBC mode with PKCS7 padding",
            ),
            ([0xaa; 16], [0x55; 16], &[0x42u8; 100]),
        ];

        for (key, iv, plaintext) in test_cases {
            aes_128_cbc_test(&key, &iv, plaintext);
        }
    }

    #[test]
    fn test_aes256_cbc_decryption() {
        let test_cases = [
            (
                [
                    0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe, 0x2b, 0x73, 0xae, 0xf0, 0x85,
                    0x7d, 0x77, 0x81, 0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7, 0x2d, 0x98,
                    0x10, 0xa3, 0x09, 0x14, 0xdf, 0xf4,
                ],
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f,
                ],
                b"Hello, World!" as &[u8],
            ),
            (
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
                    0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
                ],
                [0xff; 16],
                b"The quick brown fox jumps over the lazy dog",
            ),
            (
                [0xff; 32],
                [0x00; 16],
                b"AES-256 test vector for CBC mode with PKCS7 padding",
            ),
            ([0xaa; 32], [0x55; 16], &[0x42u8; 100]),
        ];

        for (key, iv, plaintext) in test_cases {
            aes_256_cbc_test(&key, &iv, plaintext);
        }
    }

    #[test]
    fn test_edge_cases() {
        let key128 = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let key256 = [
            0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe, 0x2b, 0x73, 0xae, 0xf0, 0x85, 0x7d,
            0x77, 0x81, 0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7, 0x2d, 0x98, 0x10, 0xa3,
            0x09, 0x14, 0xdf, 0xf4,
        ];
        let iv = [0x00; 16];

        aes_128_cbc_test(&key128, &iv, b"");
        aes_256_cbc_test(&key256, &iv, b"");

        aes_128_cbc_test(&key128, &iv, b"a");
        aes_256_cbc_test(&key256, &iv, b"a");

        aes_128_cbc_test(&key128, &iv, &[0xffu8; 15]);
        aes_256_cbc_test(&key256, &iv, &[0xffu8; 15]);

        aes_128_cbc_test(&key128, &iv, &[0xffu8; 16]);
        aes_256_cbc_test(&key256, &iv, &[0xffu8; 16]);

        aes_128_cbc_test(&key128, &iv, &[0xffu8; 17]);
        aes_256_cbc_test(&key256, &iv, &[0xffu8; 17]);
    }

    #[test]
    fn test_invalid_key_lengths() {
        assert!(AES128Cipher::new(&[0u8; 15]).is_none());
        assert!(AES128Cipher::new(&[0u8; 17]).is_none());
        assert!(AES256Cipher::new(&[0u8; 31]).is_none());
        assert!(AES256Cipher::new(&[0u8; 33]).is_none());
    }
}
