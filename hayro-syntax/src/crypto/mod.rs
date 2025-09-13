use crate::crypto::rc4::Rc4;
use crate::object;
use crate::object::dict::keys::{FILTER, LENGTH, O, P, R, U, V};
use crate::object::{Dict, Name, Object, ObjectIdentifier};
use std::borrow::Cow;
use std::ops::Deref;

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

#[derive(Debug)]
pub(crate) enum Decryptor {
    None,
    Rc4(Vec<u8>),
}

impl Decryptor {
    pub(crate) fn decrypt(&self, id: ObjectIdentifier, data: &[u8]) -> Option<Vec<u8>> {
        match self {
            Decryptor::None => Some(data.to_vec()),
            Decryptor::Rc4(r) => {
                let n = r.len();

                // Algorithm 1:
                // a) Obtain the object number and generation number from the object identifier of
                // the string or stream to be encrypted (see 7.3.10, "Indirect objects"). If the
                // string is a direct object, use the identifier of the indirect object containing
                // it.
                let mut key = r.clone();

                // b) For all strings and streams without crypt filter specifier; treating the
                // object number and generation number as binary integers, extend the original
                // n-byte file encryption key to n + 5 bytes by appending the low-order 3 bytes of
                // the object number and the low-order 2 bytes of the generation number in that
                // order, low-order byte first.
                key.extend(&id.obj_num.to_le_bytes()[..3]); // Low 3 bytes
                key.extend(&id.gen_num.to_le_bytes()[..2]); // Low 2 bytes

                // c) Initialise the MD5 hash function and pass the result of step (b) as input
                // to this function.
                let hash = md5::calculate(&key);

                // d) Use the first (n + 5) bytes, up to a maximum of 16, of the output
                // from the MD5 hash as the key for the RC4 or AES symmetric key algorithms,
                // along with the string or stream data to be encrypted.
                let final_key = &hash[..std::cmp::min(16, n + 5)];

                let mut rc = Rc4::new(final_key);
                Some(rc.decrypt(data))
            }
        }
    }
}

const DEFAULT_USER_PASSWORD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

pub(crate) struct CryptoDict {
    algorithm: Decryptor,
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
    let revision = dict
        .get::<u8>(R)
        .ok_or(DecryptionError::InvalidEncryption)?;
    let algorithm = match encryption_v {
        1 => Decryptor::Rc4,
        2 => {
            let length = dict.get::<u32>(LENGTH).unwrap_or(40);

            Decryptor::Rc4
        }
        _ => {
            return Err(DecryptionError::UnsupportedAlgorithm);
        }
    };

    let length = match encryption_v {
        1 => 40,
        2 => dict.get::<u32>(LENGTH).unwrap_or(40),
        _ => unreachable!(),
    }
    .min(128);

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

            // f) TODO: (Security handlers of revision 4 or greater) If document metadata
            // is not being encrypted, pass 4 bytes with the value 0xFFFFFFFF to the MD5 hash function.

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
        2 => {
            let mut rc = Rc4::new(&decryption_key);
            let encrypted = rc.decrypt(&DEFAULT_USER_PASSWORD);

            if encrypted.as_slice() != user_password.get().as_ref() {
                return Err(DecryptionError::PasswordProtected);
            }
        }
        _ => unimplemented!(),
    }

    Ok(Decryptor::Rc4(decryption_key))
}
