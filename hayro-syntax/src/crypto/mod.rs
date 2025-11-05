//! Cryptographic implementations for hayro, ported from pdf.js.
//!
//! **Important note**: Please keep in mind that these haven't been
//! audited and should not be used for security-critical purposes, like creating new
//! encrypted PDFs. They solely serve the purpose of being able to decrypt and read
//! _already_ encrypted documents, where security isn't really relevant.

use crate::crypto::DecryptionError::InvalidEncryption;
use crate::crypto::aes::{AES128Cipher, AES256Cipher};
use crate::crypto::rc4::Rc4;
use crate::object;
use crate::object::dict::keys::{
    CF, CFM, ENCRYPT_META_DATA, FILTER, LENGTH, O, OE, P, R, STM_F, STR_F, U, UE, V,
};
use crate::object::{Dict, Name, ObjectIdentifier};
use std::collections::HashMap;
use std::ops::Deref;

mod aes;
mod md5;
mod rc4;
mod sha256;
mod sha384;
mod sha512;

const PASSWORD_PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

const PASSWORD: &[u8; 0] = b"";

/// An error that occurred during decryption.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DecryptionError {
    /// The ID entry is missing in the PDF.
    MissingIDEntry,
    /// The PDF is password-protected (currently not supported).
    PasswordProtected,
    /// The PDF has invalid encryption.
    InvalidEncryption,
    /// The PDF uses an unsupported encryption algorithm.
    UnsupportedAlgorithm,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum DecryptorTag {
    None,
    Rc4,
    Aes128,
    Aes256,
}

impl DecryptorTag {
    fn from_name(name: &Name) -> Option<Self> {
        match name.as_str() {
            "None" | "Identity" => Some(Self::None),
            "V2" => Some(Self::Rc4),
            "AESV2" => Some(Self::Aes128),
            "AESV3" => Some(Self::Aes256),
            _ => None,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Decryptor {
    None,
    Rc4 { key: Vec<u8> },
    Aes128 { key: Vec<u8>, dict: DecryptorData },
    Aes256 { key: Vec<u8>, dict: DecryptorData },
}

#[derive(Debug, Copy, Clone)]
pub(crate) enum DecryptionTarget {
    String,
    Stream,
}

impl Decryptor {
    pub(crate) fn decrypt(
        &self,
        id: ObjectIdentifier,
        data: &[u8],
        target: DecryptionTarget,
    ) -> Option<Vec<u8>> {
        match self {
            Decryptor::None => Some(data.to_vec()),
            Decryptor::Rc4 { key } => decrypt_rc4(key, data, id),
            Decryptor::Aes128 { key, dict } | Decryptor::Aes256 { key, dict } => {
                let crypt_dict = match target {
                    DecryptionTarget::String => dict.string_filter,
                    DecryptionTarget::Stream => dict.stream_filter,
                };

                match crypt_dict.cfm {
                    DecryptorTag::None => Some(data.to_vec()),
                    DecryptorTag::Rc4 => decrypt_rc4(key, data, id),
                    DecryptorTag::Aes128 => decrypt_aes128(key, data, id),
                    DecryptorTag::Aes256 => decrypt_aes256(key, data),
                }
            }
        }
    }
}

pub(crate) fn get(dict: &Dict, id: &[u8]) -> Result<Decryptor, DecryptionError> {
    let filter = dict.get::<Name>(FILTER).ok_or(InvalidEncryption)?;

    if filter.deref() != b"Standard" {
        return Err(DecryptionError::UnsupportedAlgorithm);
    }

    let encryption_v = dict.get::<u8>(V).ok_or(InvalidEncryption)?;
    let encrypt_metadata = dict.get::<bool>(ENCRYPT_META_DATA).unwrap_or(true);
    let revision = dict.get::<u8>(R).ok_or(InvalidEncryption)?;
    let length = match encryption_v {
        1 => 40,
        2 => dict.get::<u16>(LENGTH).unwrap_or(40),
        4 => dict.get::<u16>(LENGTH).unwrap_or(128),
        5 => 256,
        _ => return Err(DecryptionError::UnsupportedAlgorithm),
    };

    let (algorithm, data) = match encryption_v {
        1 => (DecryptorTag::Rc4, None),
        2 => (DecryptorTag::Rc4, None),
        4 => (
            DecryptorTag::Aes128,
            Some(DecryptorData::from_dict(dict, length).ok_or(InvalidEncryption)?),
        ),
        5 | 6 => (
            DecryptorTag::Aes256,
            Some(DecryptorData::from_dict(dict, length).ok_or(InvalidEncryption)?),
        ),
        _ => {
            return Err(DecryptionError::UnsupportedAlgorithm);
        }
    };

    let byte_length = length / 8;

    let owner_string = dict.get::<object::String>(O).ok_or(InvalidEncryption)?;
    let user_string = dict.get::<object::String>(U).ok_or(InvalidEncryption)?;
    let permissions = {
        let raw = dict.get::<i64>(P).ok_or(InvalidEncryption)?;

        if raw < 0 {
            u32::from_be_bytes((raw as i32).to_be_bytes())
        } else {
            raw as u32
        }
    };

    let mut decryption_key = if revision <= 4 {
        let key = decryption_key_rev1234(
            encrypt_metadata,
            revision,
            byte_length,
            &owner_string,
            permissions,
            id,
        )?;
        authenticate_user_password_rev234(revision, &key, id, &user_string)?;

        key
    } else {
        decryption_key_rev56(dict, revision, &owner_string, &user_string)?
    };

    // See pdf.js issue 19484.
    if encryption_v == 4 && decryption_key.len() < 16 {
        decryption_key.resize(16, 0)
    }

    match algorithm {
        DecryptorTag::None => Ok(Decryptor::None),
        DecryptorTag::Rc4 => Ok(Decryptor::Rc4 {
            key: decryption_key,
        }),
        DecryptorTag::Aes128 => Ok(Decryptor::Aes128 {
            key: decryption_key,
            dict: data.unwrap(),
        }),
        DecryptorTag::Aes256 => Ok(Decryptor::Aes256 {
            key: decryption_key,
            dict: data.unwrap(),
        }),
    }
}

/// Algorithm 1.A: Encryption of data using the AES algorithms
fn decrypt_aes256(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    // a) Use the 32-byte file encryption key for the AES-256 symmetric key algorithm,
    // along with the string or stream data to be encrypted.
    // Use the AES algorithm in Cipher Block Chaining (CBC) mode, which requires an initialization
    // vector. The block size parameter is set to 16 bytes, and the initialization
    // vector is a 16-byte random number that is stored as the first 16 bytes of the
    // encrypted stream or string.
    let (iv, data) = data.split_at_checked(16)?;
    let iv: [u8; 16] = iv.try_into().ok()?;
    let cipher = AES256Cipher::new(key)?;
    Some(cipher.decrypt_cbc(data, &iv, true))
}

fn decrypt_aes128(key: &[u8], data: &[u8], id: ObjectIdentifier) -> Option<Vec<u8>> {
    decrypt_rc_aes(key, id, true, |key| {
        // If using the AES algorithm, the Cipher Block Chaining (CBC) mode, which requires an initialization
        // vector, is used. The block size parameter is set to 16 bytes, and the initialization vector is a 16-byte
        // random number that is stored as the first 16 bytes of the encrypted stream or string.
        let cipher = AES128Cipher::new(key)?;
        let (iv, data) = data.split_at_checked(16)?;
        let iv: [u8; 16] = iv.try_into().ok()?;

        Some(cipher.decrypt_cbc(data, &iv, true))
    })
}

fn decrypt_rc4(key: &[u8], data: &[u8], id: ObjectIdentifier) -> Option<Vec<u8>> {
    decrypt_rc_aes(key, id, false, |key| {
        let mut rc = Rc4::new(key);
        Some(rc.decrypt(data))
    })
}

/// Algorithm 1: Encryption of data using the RC4 or AES algorithms
fn decrypt_rc_aes(
    key: &[u8],
    id: ObjectIdentifier,
    aes: bool,
    with_key: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let n = key.len();
    // a) Obtain the object number and generation number from the object identifier of
    // the string or stream to be encrypted (see 7.3.10, "Indirect objects"). If the
    // string is a direct object, use the identifier of the indirect object containing
    // it.
    let mut key = key.to_vec();

    // b) For all strings and streams without crypt filter specifier; treating the
    // object number and generation number as binary integers, extend the original
    // n-byte file encryption key to n + 5 bytes by appending the low-order 3 bytes of
    // the object number and the low-order 2 bytes of the generation number in that
    // order, low-order byte first.
    key.extend(&id.obj_num.to_le_bytes()[..3]);
    key.extend(&id.gen_num.to_le_bytes()[..2]);

    // If using the AES algorithm, extend the file encryption key an additional 4 bytes by adding the value
    // "sAlT", which corresponds to the hexadecimal values 0x73, 0x41, 0x6C, 0x54. (This addition is done
    // for backward compatibility and is not intended to provide additional security.)
    if aes {
        key.extend(b"sAlT")
    }

    // c) Initialise the MD5 hash function and pass the result of step (b) as input
    // to this function.
    let hash = md5::calculate(&key);

    // d) Use the first (n + 5) bytes, up to a maximum of 16, of the output
    // from the MD5 hash as the key for the RC4 or AES symmetric key algorithms,
    // along with the string or stream data to be encrypted.
    let final_key = &hash[..std::cmp::min(16, n + 5)];

    with_key(final_key)
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct DecryptorData {
    stream_filter: CryptDictionary,
    string_filter: CryptDictionary,
}

impl DecryptorData {
    fn from_dict(dict: &Dict, default_length: u16) -> Option<Self> {
        let mut mappings = HashMap::new();

        if let Some(dict) = dict.get::<Dict>(CF) {
            for key in dict.keys() {
                if let Some(dict) = dict.get::<Dict>(key.clone())
                    && let Some(crypt_dict) = CryptDictionary::from_dict(&dict, default_length)
                {
                    mappings.insert(key.as_str().to_string(), crypt_dict);
                }
            }
        }

        let stm_f = *mappings
            .get(dict.get::<Name>(STM_F)?.as_str())
            .unwrap_or(&CryptDictionary::identity(default_length));
        let str_f = *mappings
            .get(dict.get::<Name>(STR_F)?.as_str())
            .unwrap_or(&CryptDictionary::identity(default_length));

        Some(Self {
            stream_filter: stm_f,
            string_filter: str_f,
        })
    }
}

#[derive(Debug, Copy, Clone)]
struct CryptDictionary {
    cfm: DecryptorTag,
    _length: u16,
}

impl CryptDictionary {
    fn from_dict(dict: &Dict, default_length: u16) -> Option<Self> {
        let cfm = DecryptorTag::from_name(&dict.get::<Name>(CFM)?)?;
        // The standard security handler expresses the Length entry in bytes (e.g., 32 means a
        // length of 256 bits) and public-key security handlers express it as is (e.g., 256 means a
        // length of 256 bits).
        // Note: We only support the standard security handler.
        let mut length = dict.get::<u16>(LENGTH).unwrap_or(default_length / 8);

        // When CFM is AESV2, the Length key shall have the value of 128. When
        // CFM is AESV3, the Length key shall have a value of 256.
        if cfm == DecryptorTag::Aes128 {
            length = 16;
        } else if cfm == DecryptorTag::Aes256 {
            length = 32;
        }

        Some(CryptDictionary {
            cfm,
            _length: length,
        })
    }

    fn identity(default_length: u16) -> CryptDictionary {
        Self {
            cfm: DecryptorTag::None,
            _length: default_length,
        }
    }
}

/// Algorithm 2.B: Computing a hash (revision 6 and later)
fn compute_hash_rev56(
    password: &[u8],
    validation_salt: &[u8],
    user_string: Option<&[u8]>,
    revision: u8,
) -> Result<[u8; 32], DecryptionError> {
    // Take the SHA-256 hash of the original input to the algorithm and name the resulting
    // 32 bytes, K.
    let mut k = {
        let mut input = Vec::new();
        input.extend_from_slice(password);
        input.extend_from_slice(validation_salt);

        if let Some(user_string) = user_string {
            input.extend_from_slice(user_string);
        }

        let hash = sha256::calculate(&input);

        // Apparently revision 5 only uses this hash.
        if revision == 5 {
            return Ok(hash);
        }

        hash.to_vec()
    };

    let mut round: u16 = 0;

    // Perform the following steps (a)-(d) 64 times:
    loop {
        // a) Make a new string, K1, consisting of 64 repetitions of the sequence:
        // input password, K, the 48-byte user key. The 48 byte user key is only used when
        // checking the owner password or creating the owner key. If checking the user
        // password or creating the user key, K1 is the concatenation of the input
        // password and K.
        let k1 = {
            let mut single: Vec<u8> = vec![];
            single.extend(password);
            single.extend(&k);

            if let Some(user_string) = user_string {
                single.extend(user_string);
            }

            single.repeat(64)
        };

        // b) Encrypt K1 with the AES-128 (CBC, no padding) algorithm,
        // using the first 16 bytes of K as the key and the second 16 bytes of K as the
        // initialization vector. The result of this encryption is E.
        let e = {
            let aes = AES128Cipher::new(&k[..16]).ok_or(InvalidEncryption)?;
            let mut res = aes.encrypt_cbc(&k1, &k[16..32].try_into().unwrap());

            // Remove padding that was added by `encrypt_cbc`.
            res.truncate(k1.len());

            res
        };

        // c) Taking the first 16 bytes of E as an unsigned big-endian integer,
        // compute the remainder, modulo 3. If the result is 0, the next hash used is
        // SHA-256, if the result is 1, the next hash used is SHA-384, if the result is
        // 2, the next hash used is SHA-512.
        let num = u128::from_be_bytes(e[..16].try_into().unwrap()) % 3;

        // d) Using the hash algorithm determined in step c, take the hash of E.
        // The result is a new value of K, which will be 32, 48, or 64 bytes in length.
        k = match num {
            0 => sha256::calculate(&e).to_vec(),
            1 => sha384::calculate(&e).to_vec(),
            2 => sha512::calculate(&e).to_vec(),
            _ => unreachable!(),
        };

        round += 1;

        // Repeat the process (a-d) with this new value for K. Following 64 rounds
        // (round number 0 to round number 63), do the following, starting with round
        // number 64:
        if round > 63 {
            // e) Look at the very last byte of E. If the value of that byte
            // (taken as an unsigned integer) is greater than the round number - 32,
            // repeat steps (a-d) again.
            let last_byte = *e.last().unwrap();

            // f) Repeat from steps (a-e) until the value of the last byte
            // is < (round number) - 32.
            // For some reason we need to use <= here?
            if (last_byte as u16) <= round - 32 {
                break;
            }
        }
    }

    // The first 32 bytes of the final K are the output of the algorithm.
    let mut result = [0u8; 32];
    result.copy_from_slice(&k[..32]);
    Ok(result)
}

/// Algorithm 2: Computing a file encryption key in order to encrypt a document (revision 4 and earlier)
fn decryption_key_rev1234(
    encrypt_metadata: bool,
    revision: u8,
    byte_length: u16,
    owner_string: &object::String,
    permissions: u32,
    id: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    let mut md5_input = vec![];

    // a) Convert password to PDFDocEncoding.
    let password = PASSWORD_PADDING;

    // b) Initialise the MD5 hash function and pass the
    // result of step a) as input to this function.
    md5_input.extend(&password);

    // c) Pass the value of the encryption dictionary's O entry
    // to the MD5 hash function.
    md5_input.extend(owner_string.get().as_ref());

    // d) Convert the integer value of the P entry to a 32-bit unsigned
    // binary number and pass these bytes to the MD5 hash function, low-order byte first.
    md5_input.extend(permissions.to_le_bytes());

    // e) Pass the first element of the file's file identifier array to the MD5 hash function.
    md5_input.extend(id);

    // f) (Security handlers of revision 4 or greater) If document metadata
    // is not being encrypted, pass 4 bytes with the value 0xFFFFFFFF to the MD5 hash function.
    if !encrypt_metadata && revision >= 4 {
        md5_input.extend(&[0xff, 0xff, 0xff, 0xff])
    }

    // g) Finish the hash.
    let mut hash = md5::calculate(&md5_input);

    // h) For revisions >= 3, do the following 50 times: Take the output from the previous
    // MD5 hash and pass the first n bytes of the output as input into a new MD5 hash,
    // where n is the number of bytes of the file encryption key as defined by the value
    // of the encryption dictionary's `Length` entry.
    if revision >= 3 {
        for _ in 0..50 {
            hash = md5::calculate(&hash[..byte_length as usize]);
        }
    }

    let decryption_key = hash[..byte_length as usize].to_vec();
    Ok(decryption_key)
}

/// Algorithm 6: Authenticating the user password
fn authenticate_user_password_rev234(
    revision: u8,
    decryption_key: &[u8],
    id: &[u8],
    user_string: &object::String,
) -> Result<(), DecryptionError> {
    // a) Perform all but the last step of Algorithm 4 (revision 2) or Algorithm 5 (revision 3 + 4).
    let result = match revision {
        2 => user_password_rev2(decryption_key),
        3 | 4 => user_password_rev34(decryption_key, id),
        _ => return Err(DecryptionError::InvalidEncryption),
    };

    // b) If the result of step (a) is equal to the value of the encryption dictionary's
    // U entry (comparing on the first 16 bytes in the case of security handlers of
    // revision 3 or greater), the password supplied is the correct user password.
    match revision {
        2 => {
            if result.as_slice() != user_string.get().as_ref() {
                return Err(DecryptionError::PasswordProtected);
            }
        }
        3 | 4 => {
            if Some(&result[..16]) != user_string.get().as_ref().get(0..16) {
                return Err(DecryptionError::PasswordProtected);
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

/// Algorithm 4: Computing the encryption dictionary’s U-entry value
/// (Security handlers of revision 2).
fn user_password_rev2(decryption_key: &[u8]) -> Vec<u8> {
    // a) Create a file encryption key based on the user password string.
    // b) Encrypt the 32-byte padding string using an RC4 encryption
    // function with the file encryption key from the preceding step.
    let mut rc = Rc4::new(decryption_key);
    rc.decrypt(&PASSWORD_PADDING)
}

/// Algorithm 5: Computing the encryption dictionary’s U (user password)
/// value (Security handlers of revision 3 or 4).
fn user_password_rev34(decryption_key: &[u8], id: &[u8]) -> Vec<u8> {
    // a) Create a file encryption key based on the user password string.
    let mut rc = Rc4::new(decryption_key);

    let mut input = vec![];
    // b) Initialise the MD5 hash function and pass the 32-byte padding string.
    input.extend(PASSWORD_PADDING);

    // c) Pass the first element of the file's file identifier array to the hash function
    // and finish the hash.
    input.extend(id);
    let hash = md5::calculate(&input);

    // d) Encrypt the 16-byte result of the hash, using an RC4 encryption function with
    // the encryption key from step (a).
    let mut encrypted = rc.encrypt(&hash);

    // e) Do the following 19 times: Take the output from the previous invocation of the
    // RC4 function and pass it as input to a new invocation of the function; use a file
    // encryption key generated by taking each byte of the original file encryption key
    // obtained in step (a) and performing an XOR (exclusive or) operation between that
    // byte and the single-byte value of the iteration counter (from 1 to 19).
    for i in 1..=19 {
        let mut key = decryption_key.to_vec();
        for byte in &mut key {
            *byte ^= i;
        }

        let mut rc = Rc4::new(&key);
        encrypted = rc.encrypt(&encrypted);
    }

    encrypted.resize(32, 0);
    encrypted
}

/// Algorithm 2.A: Retrieving the file encryption key from an encrypted document in order to decrypt it (revision 6 and later)
fn decryption_key_rev56(
    dict: &Dict,
    revision: u8,
    owner_string: &object::String,
    user_string: &object::String,
) -> Result<Vec<u8>, DecryptionError> {
    // a) The UTF-8 password string shall be generated from Unicode input by processing the input string with
    // the SASLprep (Internet RFC 4013) profile of stringprep (Internet RFC 3454) using the Normalize and BiDi
    // options, and then converting to a UTF-8 representation.
    // b) Truncate the UTF-8 representation to 127 bytes if it is longer than 127 bytes.

    let string_len = if revision <= 4 { 32 } else { 48 };

    let os = owner_string.get();
    let trimmed_os = os.get(..string_len).ok_or(InvalidEncryption)?;

    let (owner_hash, owner_tail) = trimmed_os.split_at_checked(32).ok_or(InvalidEncryption)?;
    let (owner_validation_salt, owner_key_salt) =
        owner_tail.split_at_checked(8).ok_or(InvalidEncryption)?;

    let us = user_string.get();
    let trimmed_us = us.get(..string_len).ok_or(InvalidEncryption)?;
    let (user_hash, user_tail) = trimmed_us.split_at_checked(32).ok_or(InvalidEncryption)?;
    let (user_validation_salt, user_key_salt) =
        user_tail.split_at_checked(8).ok_or(InvalidEncryption)?;

    // c) Test the password against the owner key by computing a hash using algorithm 2.B
    // with an input string consisting of the UTF-8 password concatenated with the 8 bytes of
    // owner Validation Salt, concatenated with the 48-byte U string. If the 32-byte result
    // matches the first 32 bytes of the O string, this is the owner password.
    if compute_hash_rev56(PASSWORD, owner_validation_salt, Some(trimmed_us), revision)?
        == owner_hash
    {
        // d) Compute an intermediate owner key by computing a hash using algorithm 2.B with an input string
        // consisting of the UTF-8 owner password concatenated with the 8 bytes of owner Key Salt,
        // concatenated with the 48-byte U string. The 32-byte result is the key used to decrypt the 32-byte OE string
        // using AES-256 in CBC mode with no padding and an initialization vector of zero. The 32-byte result is the file encryption key.
        let intermediate_owner_key =
            compute_hash_rev56(PASSWORD, owner_key_salt, Some(trimmed_us), revision)?;

        let oe_string = dict
            .get::<object::String>(OE)
            .ok_or(DecryptionError::InvalidEncryption)?;

        if oe_string.get().len() != 32 {
            return Err(DecryptionError::InvalidEncryption);
        }

        let cipher = AES256Cipher::new(&intermediate_owner_key).ok_or(InvalidEncryption)?;
        let zero_iv = [0u8; 16];

        Ok(cipher.decrypt_cbc(&oe_string.get(), &zero_iv, false))
    } else if compute_hash_rev56(PASSWORD, user_validation_salt, None, revision)? == user_hash {
        // e) Compute an intermediate user key by computing a hash using algorithm 2.B with an input string
        // consisting of the UTF-8 user password concatenated with the 8 bytes of user Key Salt. The 32-byte result
        // is the key used to decrypt the 32-byte UE string using AES-256 in CBC mode with no padding and an
        // initialization vector of zero. The 32-byte result is the file encryption key.
        let intermediate_key = compute_hash_rev56(PASSWORD, user_key_salt, None, revision)?;

        let ue_string = dict.get::<object::String>(UE).ok_or(InvalidEncryption)?;

        if ue_string.get().len() != 32 {
            return Err(InvalidEncryption);
        }

        let cipher = AES256Cipher::new(&intermediate_key).ok_or(InvalidEncryption)?;
        let zero_iv = [0u8; 16];

        Ok(cipher.decrypt_cbc(&ue_string.get(), &zero_iv, false))
    } else {
        Err(DecryptionError::PasswordProtected)
    }

    // TODO:
    // f) Decrypt the 16-byte Perms string using AES-256 in ECB mode with an initialization vector of zero and
    // the file encryption key as the key. Verify that bytes 9-11 of the result are the characters "a", "d",
    // "b". Bytes 0-3 of the decrypted Perms entry, treated as a little-endian integer, are the user
    // permissions. They shall match the value in the P key.
}
