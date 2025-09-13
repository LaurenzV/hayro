use crate::crypto::algo::Algorithm;
use crate::object;
use crate::object::dict::keys::{FILTER, LENGTH, O, P, R, U, V};
use crate::object::{Dict, Name};
use log::warn;
use std::ops::Deref;

mod algo;
mod md5;
mod rc4;

pub enum DecryptionError {
    MissingIDEntry,
}

const DEFAULT_USER_PASSWORD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

pub(crate) struct CryptoDict {
    algorithm: Algorithm,
}

pub(crate) fn get(dict: &Dict, id: &[u8]) -> Option<Algorithm> {
    let filter = dict.get::<Name>(FILTER)?;

    if filter.deref() != b"Standard" {
        warn!(
            "Non-standard encryption filter {} is unsupported",
            filter.as_str()
        );

        return None;
    }

    let encryption_v = dict.get::<u8>(V)?;
    let revision = dict.get::<u8>(R)?;
    let algorithm = match encryption_v {
        1 => Algorithm::Rc4,
        2 => {
            let length = dict.get::<u32>(LENGTH).unwrap_or(40);

            Algorithm::Rc4
        }
        _ => {
            warn!("Unsupported encryption method with number {encryption_v}");

            return None;
        }
    };

    let length = match encryption_v {
        1 => 40,
        2 => dict.get::<u32>(LENGTH).unwrap_or(40),
        _ => unreachable!(),
    }
    .min(128);

    let byte_length = length / 8;

    let owner_password = dict.get::<object::String>(O)?;
    let user_password = dict.get::<object::String>(U)?;
    let permissions = dict.get::<u32>(P)?;

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

    Some(algorithm)
}
