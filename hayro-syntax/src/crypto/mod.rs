//! Ported from pdf.js.

use crate::crypto::DecryptionError::InvalidEncryption;
use crate::crypto::aes::{AES128Cipher, AES256Cipher};
use crate::crypto::rc4::Rc4;
use crate::object;
use crate::object::dict::keys::{
    CF, CFM, ENCRYPT_META_DATA, FILTER, IDENTITY, LENGTH, O, OE, P, R, STM_F, STR_F, U, UE, V,
};
use crate::object::{Dict, Name, ObjectIdentifier};
use crate::sync::HashMap;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp;
use core::ops::Deref;

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

/// An error that occurred during decryption.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DecryptionError {
    /// The ID entry is missing in the PDF.
    MissingIDEntry,
    /// No password or a wrong one was provided.
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
    fn from_name(name: &Name<'_>) -> Option<Self> {
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
pub(crate) enum DecryptionTarget<'a> {
    String,
    Stream,
    Named(&'a [u8]),
}

impl Decryptor {
    pub(crate) fn decrypt(
        &self,
        id: ObjectIdentifier,
        data: &[u8],
        target: DecryptionTarget<'_>,
    ) -> Option<Vec<u8>> {
        if matches!(target, DecryptionTarget::Named(IDENTITY)) {
            return Some(data.to_vec());
        }

        match self {
            Self::None => Some(data.to_vec()),
            Self::Rc4 { key } => match target {
                DecryptionTarget::Named(_) => None,
                _ => decrypt_rc4(key, data, id),
            },
            Self::Aes128 { key, dict } | Self::Aes256 { key, dict } => {
                let crypt_dict = match target {
                    DecryptionTarget::String => dict.string_filter,
                    DecryptionTarget::Stream => dict.stream_filter,
                    DecryptionTarget::Named(name) => *dict.crypt_filters.get(name)?,
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

pub(crate) fn get(
    dict: &Dict<'_>,
    id: &[u8],
    password: &[u8],
) -> Result<Decryptor, DecryptionError> {
    let filter = dict.get::<Name<'_>>(FILTER).ok_or(InvalidEncryption)?;

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
    if byte_length == 0 {
        return Err(InvalidEncryption);
    }

    let owner_string = dict.get::<object::String<'_>>(O).ok_or(InvalidEncryption)?;
    let user_string = dict.get::<object::String<'_>>(U).ok_or(InvalidEncryption)?;
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
            password,
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
        decryption_key_rev56(dict, revision, password, &owner_string, &user_string)?
    };

    // See pdf.js issue 19484.
    if encryption_v == 4 && decryption_key.len() < 16 {
        decryption_key.resize(16, 0);
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

fn decrypt_aes256(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let (iv, data) = data.split_at_checked(16)?;
    let iv: [u8; 16] = iv.try_into().ok()?;
    let cipher = AES256Cipher::new(key)?;
    Some(cipher.decrypt_cbc(data, &iv, true))
}

fn decrypt_aes128(key: &[u8], data: &[u8], id: ObjectIdentifier) -> Option<Vec<u8>> {
    decrypt_rc_aes(key, id, true, |key| {
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

fn decrypt_rc_aes(
    key: &[u8],
    id: ObjectIdentifier,
    aes: bool,
    with_key: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let n = key.len();
    let mut key = key.to_vec();

    key.extend(&id.obj_number.to_le_bytes()[..3]);
    key.extend(&id.gen_number.to_le_bytes()[..2]);

    if aes {
        key.extend(b"sAlT");
    }

    let hash = md5::calculate(&key);

    let final_key = &hash[..cmp::min(16, n + 5)];

    with_key(final_key)
}

#[derive(Debug, Clone)]
pub(crate) struct DecryptorData {
    stream_filter: CryptDictionary,
    string_filter: CryptDictionary,
    crypt_filters: HashMap<Vec<u8>, CryptDictionary>,
}

impl DecryptorData {
    fn from_dict(dict: &Dict<'_>, default_length: u16) -> Option<Self> {
        let mut mappings = HashMap::new();

        if let Some(dict) = dict.get::<Dict<'_>>(CF) {
            for key in dict.keys() {
                if let Some(dict) = dict.get::<Dict<'_>>(key.as_ref())
                    && let Some(crypt_dict) = CryptDictionary::from_dict(&dict, default_length)
                {
                    mappings.insert(key.to_vec(), crypt_dict);
                }
            }
        }

        let stm_f = *mappings
            .get(dict.get::<Name<'_>>(STM_F)?.deref())
            .unwrap_or(&CryptDictionary::identity(default_length));
        let str_f = *mappings
            .get(dict.get::<Name<'_>>(STR_F)?.deref())
            .unwrap_or(&CryptDictionary::identity(default_length));

        Some(Self {
            stream_filter: stm_f,
            string_filter: str_f,
            crypt_filters: mappings,
        })
    }
}

#[derive(Debug, Copy, Clone)]
struct CryptDictionary {
    cfm: DecryptorTag,
    _length: u16,
}

impl CryptDictionary {
    fn from_dict(dict: &Dict<'_>, default_length: u16) -> Option<Self> {
        let cfm = DecryptorTag::from_name(&dict.get::<Name<'_>>(CFM)?)?;
        // `Length` is in bytes for the standard security handler.
        let mut length = dict.get::<u16>(LENGTH).unwrap_or(default_length / 8);

        if cfm == DecryptorTag::Aes128 {
            length = 16;
        } else if cfm == DecryptorTag::Aes256 {
            length = 32;
        }

        Some(Self {
            cfm,
            _length: length,
        })
    }

    fn identity(default_length: u16) -> Self {
        Self {
            cfm: DecryptorTag::None,
            _length: default_length,
        }
    }
}

fn compute_hash_rev56(
    password: &[u8],
    validation_salt: &[u8],
    user_string: Option<&[u8]>,
    revision: u8,
) -> Result<[u8; 32], DecryptionError> {
    let mut k = {
        let mut input = Vec::new();
        input.extend_from_slice(password);
        input.extend_from_slice(validation_salt);

        if let Some(user_string) = user_string {
            input.extend_from_slice(user_string);
        }

        let hash = sha256::calculate(&input);

        // Revision 5 only uses the initial hash.
        if revision == 5 {
            return Ok(hash);
        }

        hash.to_vec()
    };

    let mut round: u16 = 0;

    loop {
        let k1 = {
            let mut single: Vec<u8> = vec![];
            single.extend(password);
            single.extend(&k);

            if let Some(user_string) = user_string {
                single.extend(user_string);
            }

            single.repeat(64)
        };

        let e = {
            let aes = AES128Cipher::new(&k[..16]).ok_or(InvalidEncryption)?;
            let mut res = aes.encrypt_cbc(&k1, &k[16..32].try_into().unwrap());

            res.truncate(k1.len());

            res
        };

        let num = u128::from_be_bytes(e[..16].try_into().unwrap()) % 3;

        k = match num {
            0 => sha256::calculate(&e).to_vec(),
            1 => sha384::calculate(&e).to_vec(),
            2 => sha512::calculate(&e).to_vec(),
            _ => unreachable!(),
        };

        round += 1;

        if round > 63 {
            let last_byte = *e.last().unwrap();

            if (last_byte as u16) <= round - 32 {
                break;
            }
        }
    }

    let mut result = [0_u8; 32];
    result.copy_from_slice(&k[..32]);
    Ok(result)
}

fn decryption_key_rev1234(
    password: &[u8],
    encrypt_metadata: bool,
    revision: u8,
    byte_length: u16,
    owner_string: &object::String<'_>,
    permissions: u32,
    id: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    let mut md5_input = vec![];

    // TODO: Convert to PDFDocEncoding.
    let mut padded_password = [0_u8; 32];
    let copy_len = password.len().min(32);
    padded_password[..copy_len].copy_from_slice(&password[..copy_len]);
    if copy_len < 32 {
        padded_password[copy_len..].copy_from_slice(&PASSWORD_PADDING[..(32 - copy_len)]);
    }

    md5_input.extend(&padded_password);

    md5_input.extend(owner_string.as_ref());

    md5_input.extend(permissions.to_le_bytes());

    md5_input.extend(id);

    if !encrypt_metadata && revision >= 4 {
        md5_input.extend(&[0xff, 0xff, 0xff, 0xff]);
    }

    let mut hash = md5::calculate(&md5_input);

    if byte_length as usize > hash.len() {
        return Err(InvalidEncryption);
    }

    if revision >= 3 {
        for _ in 0..50 {
            hash = md5::calculate(&hash[..byte_length as usize]);
        }
    }

    let decryption_key = hash[..byte_length as usize].to_vec();
    Ok(decryption_key)
}

fn authenticate_user_password_rev234(
    revision: u8,
    decryption_key: &[u8],
    id: &[u8],
    user_string: &object::String<'_>,
) -> Result<(), DecryptionError> {
    let result = match revision {
        2 => user_password_rev2(decryption_key),
        3 | 4 => user_password_rev34(decryption_key, id),
        _ => return Err(InvalidEncryption),
    };

    match revision {
        2 => {
            if result.as_slice() != user_string.as_ref() {
                return Err(DecryptionError::PasswordProtected);
            }
        }
        3 | 4 => {
            if Some(&result[..16]) != user_string.as_ref().get(0..16) {
                return Err(DecryptionError::PasswordProtected);
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

fn user_password_rev2(decryption_key: &[u8]) -> Vec<u8> {
    let mut rc = Rc4::new(decryption_key);
    rc.decrypt(&PASSWORD_PADDING)
}

fn user_password_rev34(decryption_key: &[u8], id: &[u8]) -> Vec<u8> {
    let mut rc = Rc4::new(decryption_key);

    let mut input = vec![];
    input.extend(PASSWORD_PADDING);

    input.extend(id);
    let hash = md5::calculate(&input);

    let mut encrypted = rc.encrypt(&hash);

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

fn decryption_key_rev56(
    dict: &Dict<'_>,
    revision: u8,
    password: &[u8],
    owner_string: &object::String<'_>,
    user_string: &object::String<'_>,
) -> Result<Vec<u8>, DecryptionError> {
    // TODO: Apply SASLprep.
    let password = &password[..password.len().min(127)];

    let string_len = if revision <= 4 { 32 } else { 48 };

    let trimmed_os = owner_string.get(..string_len).ok_or(InvalidEncryption)?;

    let (owner_hash, owner_tail) = trimmed_os.split_at_checked(32).ok_or(InvalidEncryption)?;
    let (owner_validation_salt, owner_key_salt) =
        owner_tail.split_at_checked(8).ok_or(InvalidEncryption)?;

    let trimmed_us = user_string.get(..string_len).ok_or(InvalidEncryption)?;
    let (user_hash, user_tail) = trimmed_us.split_at_checked(32).ok_or(InvalidEncryption)?;
    let (user_validation_salt, user_key_salt) =
        user_tail.split_at_checked(8).ok_or(InvalidEncryption)?;

    if compute_hash_rev56(password, owner_validation_salt, Some(trimmed_us), revision)?
        == owner_hash
    {
        let intermediate_owner_key =
            compute_hash_rev56(password, owner_key_salt, Some(trimmed_us), revision)?;

        let oe_string = dict
            .get::<object::String<'_>>(OE)
            .ok_or(InvalidEncryption)?;

        if oe_string.len() != 32 {
            return Err(InvalidEncryption);
        }

        let cipher = AES256Cipher::new(&intermediate_owner_key).ok_or(InvalidEncryption)?;
        let zero_iv = [0_u8; 16];

        Ok(cipher.decrypt_cbc(&oe_string, &zero_iv, false))
    } else if compute_hash_rev56(password, user_validation_salt, None, revision)? == user_hash {
        let intermediate_key = compute_hash_rev56(password, user_key_salt, None, revision)?;

        let ue_string = dict
            .get::<object::String<'_>>(UE)
            .ok_or(InvalidEncryption)?;

        if ue_string.len() != 32 {
            return Err(InvalidEncryption);
        }

        let cipher = AES256Cipher::new(&intermediate_key).ok_or(InvalidEncryption)?;
        let zero_iv = [0_u8; 16];

        Ok(cipher.decrypt_cbc(&ue_string, &zero_iv, false))
    } else {
        Err(DecryptionError::PasswordProtected)
    }

    // TODO: Verify Perms.
}
