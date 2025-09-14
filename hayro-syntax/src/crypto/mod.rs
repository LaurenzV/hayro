//! Cryptographic implementations for hayro, ported from pdf.js.
//!
//! **Important note**: Please keep in mind that these haven't been
//! audited and should not be used for security-critical purposes, like creating new
//! encrypted PDFs. They solely serve the purpose of being able to decrypt and read
//! _already_ encrypted documents, where security isn't really relevant.

use crate::crypto::aes::AES128Cipher;
use crate::crypto::rc4::Rc4;
use crate::object;
use crate::object::dict::keys::{
    CF, CFM, ENCRYPT_META_DATA, FILTER, LENGTH, O, P, R, STM_F, STR_F, U, V,
};
use crate::object::{Dict, Name, Object, ObjectIdentifier};
use std::collections::HashMap;
use std::ops::Deref;

mod aes;
mod algo;
mod md5;
mod rc4;

#[derive(Debug, Copy, Clone)]
pub enum DecryptionError {
    MissingIDEntry,
    PasswordProtected,
    InvalidEncryption,
    UnsupportedAlgorithm,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum DecryptorTag {
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
#[derive(Debug)]
pub(crate) enum Decryptor {
    None,
    Rc4 { key: Vec<u8> },
    Aes128 { key: Vec<u8>, dict: DecryptorData },
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
            Decryptor::Aes128 { key, dict } => {
                let crypt_dict = match target {
                    DecryptionTarget::String => dict.string_filter,
                    DecryptionTarget::Stream => dict.stream_filter
                };
                
                match crypt_dict.cfm {
                    DecryptorTag::None => Some(data.to_vec()),
                    DecryptorTag::Rc4 => decrypt_rc4(key, data, id),
                    DecryptorTag::Aes128 => decrypt_aes128(key, data, id),
                    DecryptorTag::Aes256 => unimplemented!()
                }
            }
            _ => unimplemented!(),
        }
    }
}

fn decrypt_aes128(key: &[u8], data: &[u8], id: ObjectIdentifier) -> Option<Vec<u8>> {
    key_hash(key, id, true, |key| {
        // If using the AES algorithm, the Cipher Block Chaining (CBC) mode, which requires an initialization
        // vector, is used. The block size parameter is set to 16 bytes, and the initialization vector is a 16-byte
        // random number that is stored as the first 16 bytes of the encrypted stream or string.
        let cipher = AES128Cipher::new(key).ok()?;
        let (iv, data) = data.split_at_checked(16)?;
        let iv: [u8; 16] = iv.try_into().ok()?;

        Some(cipher.decrypt_cbc(data, &iv))
    })
}

fn decrypt_rc4(key: &[u8], data: &[u8], id: ObjectIdentifier) -> Option<Vec<u8>> {
    key_hash(key, id, false, |key| {
        let mut rc = Rc4::new(key);
        Some(rc.decrypt(data))
    })
}

fn key_hash(
    key: &[u8],
    id: ObjectIdentifier,
    aes: bool,
    with_key: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let n = key.len();

    // Algorithm 1:
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

    with_key(&final_key)
}

const DEFAULT_USER_PASSWORD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

pub(crate) struct CryptoDict {
    algorithm: Decryptor,
}

#[derive(Debug, Copy, Clone)]
struct DecryptorData {
    stream_filter: CryptDictionary,
    string_filter: CryptDictionary,
}

impl DecryptorData {
    fn from_dict(dict: &Dict) -> Option<Self> {
        let mut mappings = HashMap::new();

        if let Some(dict) = dict.get::<Dict>(CF) {
            for key in dict.keys() {
                if let Some(dict) = dict.get::<Dict>(key.clone()) {
                    if let Some(crypt_dict) = CryptDictionary::from_dict(&dict) {
                        mappings.insert(key.as_str().to_string(), crypt_dict);
                    }
                }
            }
        }

        let stm_f = *mappings.get(dict.get::<Name>(STM_F)?.as_str())?;
        let str_f = *mappings.get(dict.get::<Name>(STR_F)?.as_str())?;

        Some(Self {
            stream_filter: stm_f,
            string_filter: str_f,
        })
    }
}

#[derive(Debug, Copy, Clone)]
struct CryptDictionary {
    cfm: DecryptorTag,
    length: u16,
}

impl CryptDictionary {
    fn new(tag: DecryptorTag, length: u16) -> Self {
        Self { cfm: tag, length }
    }
}

impl CryptDictionary {
    fn from_dict(dict: &Dict) -> Option<Self> {
        let cfm = DecryptorTag::from_name(&dict.get::<Name>(CFM)?)?;
        // The standard security handler expresses the Length entry in bytes (e.g., 32 means a
        // length of 256 bits) and public-key security handlers express it as is (e.g., 256 means a
        // length of 256 bits).
        // Note: We only support the standard security handler.
        let mut length = dict.get::<u16>(LENGTH)?;

        // When CFM is AESV2, the Length key shall have the value of 128. When
        // CFM is AESV3, the Length key shall have a value of 256.
        if cfm == DecryptorTag::Aes128 {
            length = 16;
        } else if cfm == DecryptorTag::Aes256 {
            length = 32;
        }

        Some(CryptDictionary { cfm, length })
    }
}

pub(crate) fn get(dict: &Dict, id: &[u8]) -> Result<Decryptor, DecryptionError> {
    let filter = dict
        .get::<Name>(FILTER)
        .ok_or(DecryptionError::InvalidEncryption)?;

    if filter.deref() != b"Standard" {
        return Err(DecryptionError::UnsupportedAlgorithm);
    }

    let encryption_v = dict
        .get::<u8>(V)
        .ok_or(DecryptionError::InvalidEncryption)?;
    let encrypt_metadata = dict.get::<bool>(ENCRYPT_META_DATA).unwrap_or(true);
    let revision = dict
        .get::<u8>(R)
        .ok_or(DecryptionError::InvalidEncryption)?;
    let length = match encryption_v {
        1 => 40,
        2 => dict.get::<u16>(LENGTH).unwrap_or(40),
        4 => 128,
        5 => 256,
        _ => unimplemented!(),
    };

    let (algorithm, data) = match encryption_v {
        1 => (DecryptorTag::Rc4, None),
        2 => (DecryptorTag::Rc4, None),
        4 => (
            DecryptorTag::Aes128,
            Some(DecryptorData::from_dict(dict).ok_or(DecryptionError::InvalidEncryption)?),
        ),
        5 => (
            DecryptorTag::Aes256,
            Some(DecryptorData::from_dict(dict).ok_or(DecryptionError::InvalidEncryption)?),
        ),
        _ => {
            return Err(DecryptionError::UnsupportedAlgorithm);
        }
    };

    let byte_length = length / 8;

    let owner_password = dict
        .get::<object::String>(O)
        .ok_or(DecryptionError::InvalidEncryption)?;
    let user_password = dict
        .get::<object::String>(U)
        .ok_or(DecryptionError::InvalidEncryption)?;
    let permissions = u32::from_be_bytes(
        dict.get::<i32>(P)
            .ok_or(DecryptionError::InvalidEncryption)?
            .to_be_bytes(),
    );

    let decryption_key = match revision {
        revision if revision <= 4 => {
            // Algorithm 2: Computing a file encryption key in order to encrypt a
            // document (revision 4 and earlier)

            let mut md5_input = vec![];

            // a) TODO: Convert password to PDFDocEncoding.
            let password = DEFAULT_USER_PASSWORD;

            // b) Initialise the MD5 hash function and pass the
            // result of step a) as input to this function.
            md5_input.extend(&password);

            // c) Pass the value of the encryption dictionary’s O entry
            // to the MD5 hash function.
            md5_input.extend(owner_password.get().as_ref());

            // d) Convert the integer value of the P entry to a 32-bit unsigned
            // binary number and pass these bytes to the MD5 hash function, low-order byte first.
            md5_input.extend(permissions.to_le_bytes());

            // e) Pass the first element of the file’s file identifier array to the MD5 hash function.
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
            // of the encryption dictionary’s `Length` entry.
            if revision >= 3 {
                for _ in 0..50 {
                    hash = md5::calculate(&hash[..byte_length as usize]);
                }
            }

            hash[..byte_length as usize].to_vec()
        }
        _ => unimplemented!(),
    };

    // Verify password
    match revision {
        _ if revision <= 4 => {
            // Algorithm 6
            // a) Perform all but the last step of Algorithm 4 (revision 2) or Algorithm 5 (revision 3 + 4).
            let result = match revision {
                2 => {
                    // Algorithm 4
                    // a) Create a file encryption key based on the user password string.
                    // b) Encrypt the 32-byte padding string using an RC4 encryption
                    // function with the file encryption key from the preceding step.
                    let mut rc = Rc4::new(&decryption_key);
                    rc.decrypt(&DEFAULT_USER_PASSWORD)
                }
                3 | 4 => {
                    // Algorithm 5
                    // a) Create a file encryption key based on the user password string.
                    let mut rc = Rc4::new(&decryption_key);

                    let mut input = vec![];
                    // b) Initialise the MD5 hash function and pass the 32-byte padding string.
                    input.extend(DEFAULT_USER_PASSWORD);

                    // c) Pass the first element of the file’s file identifier array to the hash function
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
                        let mut key = decryption_key.clone();
                        for byte in &mut key {
                            *byte = *byte ^ i;
                        }

                        let mut rc = Rc4::new(&key);
                        encrypted = rc.encrypt(&encrypted);
                    }

                    encrypted.resize(32, 0);
                    encrypted
                }
                _ => unimplemented!(),
            };

            // b) If the result of step (a) is equal to the value of the encryption dictionary’s
            // U entry (comparing on the first 16 bytes in the case of security handlers of
            // revision 3 or greater), the password supplied is the correct user password.

            match revision {
                2 => {
                    if result.as_slice() != user_password.get().as_ref() {
                        return Err(DecryptionError::PasswordProtected);
                    }
                }
                3 | 4 => {
                    if Some(&result[..16]) != user_password.get().as_ref().get(0..16) {
                        return Err(DecryptionError::PasswordProtected);
                    }
                }
                _ => unimplemented!(),
            }
        }
        _ => unimplemented!(),
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
        _ => unimplemented!(),
    }
}
