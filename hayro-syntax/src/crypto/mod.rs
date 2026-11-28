//! Ported from pdf.js.

use crate::crypto::DecryptionError::{InvalidEncryption, PasswordProtected, UnsupportedAlgorithm};
use crate::crypto::aes::{AES128Cipher, AES256Cipher};
use crate::crypto::rc4::Rc4;
use crate::object;
use crate::object::dict::keys::{
    CF, CFM, ENCRYPT_META_DATA, FILTER, IDENTITY, LENGTH, O, OE, P, R, STM_F, STR_F, U, UE, V,
};
use crate::object::{Dict, Name, ObjectIdentifier};
use crate::sync::HashMap;
use alloc::vec::Vec;
use core::ops::{Deref, RangeInclusive};

mod aes;
mod md5;
mod rc4;
mod sha256;
mod sha384;
mod sha512;

/// An error that occurred during decryption.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DecryptionError {
    /// No password or a wrong one was provided.
    PasswordProtected,
    /// The PDF has invalid encryption.
    InvalidEncryption,
    /// The PDF uses an unsupported encryption algorithm.
    UnsupportedAlgorithm,
}

impl core::fmt::Display for DecryptionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PasswordProtected => write!(f, "missing or incorrect password"),
            Self::InvalidEncryption => write!(f, "invalid encryption"),
            Self::UnsupportedAlgorithm => write!(f, "unsupported encryption algorithm"),
        }
    }
}

impl core::error::Error for DecryptionError {}

#[derive(Debug, Clone)]
pub(crate) struct Decryptor {
    key: Vec<u8>,
    strings: Cipher,
    streams: Cipher,
    crypt_filters: HashMap<Vec<u8>, Option<Cipher>>,
    plain_metadata: bool,
    metadata_id: Option<ObjectIdentifier>,
}

#[derive(Debug, Copy, Clone)]
pub(crate) enum DecryptionTarget<'a> {
    String,
    Stream,
    Named(&'a [u8]),
}

impl Decryptor {
    pub(crate) fn new(
        dict: &Dict<'_>,
        id: &[u8],
        password: &[u8],
    ) -> Result<Self, DecryptionError> {
        match Self::with_password(dict, id, password) {
            Err(PasswordProtected) if !password.is_empty() => Self::with_password(dict, id, b""),
            result => result,
        }
    }

    pub(crate) fn set_metadata_id(&mut self, id: ObjectIdentifier) {
        if self.plain_metadata {
            self.metadata_id = Some(id);
        }
    }

    pub(crate) fn decrypt(
        &self,
        id: ObjectIdentifier,
        data: &[u8],
        target: DecryptionTarget<'_>,
    ) -> Option<Vec<u8>> {
        let cipher = match target {
            DecryptionTarget::String => self.strings,
            DecryptionTarget::Stream if self.metadata_id == Some(id) => Cipher::Identity,
            DecryptionTarget::Stream => self.streams,
            DecryptionTarget::Named(IDENTITY) => Cipher::Identity,
            DecryptionTarget::Named(name) => (*self.crypt_filters.get(name)?)?,
        };

        match cipher {
            Cipher::Identity => Some(data.to_vec()),
            Cipher::Rc4 => Some(Rc4::new(&self.object_key(id, false)).decrypt(data)),
            Cipher::Aes128 => {
                let (iv, data) = data.split_first_chunk::<16>()?;
                Some(AES128Cipher::new(&self.object_key(id, true))?.decrypt_cbc(data, iv, true))
            }
            Cipher::Aes256 => {
                let (iv, data) = data.split_first_chunk::<16>()?;
                Some(AES256Cipher::new(&self.key)?.decrypt_cbc(data, iv, true))
            }
        }
    }

    fn with_password(dict: &Dict<'_>, id: &[u8], password: &[u8]) -> Result<Self, DecryptionError> {
        let filter = dict.get::<Name<'_>>(FILTER).ok_or(InvalidEncryption)?;

        if filter.deref() != b"Standard" {
            return Err(UnsupportedAlgorithm);
        }

        let version = dict.get::<u8>(V).ok_or(InvalidEncryption)?;
        let revision = dict.get::<u8>(R).ok_or(InvalidEncryption)?;
        let encrypt_metadata = dict.get::<bool>(ENCRYPT_META_DATA).unwrap_or(true);
        let length_in_bytes = |default: u16| dict.get::<u16>(LENGTH).unwrap_or(default) / 8;

        let (key_length, filters) = match version {
            1 => (5, CryptFilters::rc4()),
            2 => (length_in_bytes(40), CryptFilters::rc4()),
            4 => (length_in_bytes(128), CryptFilters::from_dict(dict)?),
            5 => (32, CryptFilters::from_dict(dict)?),
            _ => return Err(UnsupportedAlgorithm),
        };

        let owner_string = dict.get::<object::String<'_>>(O).ok_or(InvalidEncryption)?;
        let user_string = dict.get::<object::String<'_>>(U).ok_or(InvalidEncryption)?;
        let permissions = dict.get::<i64>(P).ok_or(InvalidEncryption)? as u32;

        let mut key = if revision <= 4 {
            let handler = StandardRev234 {
                revision,
                key_length: usize::from(key_length),
                owner_string: &owner_string,
                user_string: &user_string,
                permissions,
                id,
                encrypt_metadata,
            };

            handler.file_key(password)?
        } else {
            file_key_rev56(dict, revision, password, &owner_string, &user_string)?
        };

        // See pdf.js issue 19484.
        if version == 4 && key.len() < 16 {
            key.resize(16, 0);
        }

        Ok(Self {
            key,
            strings: filters.strings,
            streams: filters.streams,
            crypt_filters: filters.named,
            plain_metadata: version >= 4 && !encrypt_metadata,
            metadata_id: None,
        })
    }

    fn object_key(&self, id: ObjectIdentifier, aes: bool) -> Vec<u8> {
        let mut input = self.key.clone();
        input.extend(&id.obj_number.to_le_bytes()[..3]);
        input.extend(&id.gen_number.to_le_bytes()[..2]);

        if aes {
            input.extend(b"sAlT");
        }

        md5::calculate(&input)[..(self.key.len() + 5).min(16)].to_vec()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Cipher {
    Identity,
    Rc4,
    Aes128,
    Aes256,
}

impl Cipher {
    fn from_crypt_filter(dict: &Dict<'_>) -> Option<Self> {
        match dict.get::<Name<'_>>(CFM).as_deref() {
            None | Some(b"None") => Some(Self::Identity),
            Some(b"V2") => Some(Self::Rc4),
            Some(b"AESV2") => Some(Self::Aes128),
            Some(b"AESV3") => Some(Self::Aes256),
            Some(_) => None,
        }
    }
}

struct CryptFilters {
    strings: Cipher,
    streams: Cipher,
    named: HashMap<Vec<u8>, Option<Cipher>>,
}

impl CryptFilters {
    fn rc4() -> Self {
        Self {
            strings: Cipher::Rc4,
            streams: Cipher::Rc4,
            named: HashMap::new(),
        }
    }

    fn from_dict(dict: &Dict<'_>) -> Result<Self, DecryptionError> {
        let mut named = HashMap::new();

        if let Some(filters) = dict.get::<Dict<'_>>(CF) {
            for name in filters.keys() {
                if name.as_ref() != IDENTITY
                    && let Some(filter) = filters.get::<Dict<'_>>(name.as_ref())
                {
                    named.insert(name.to_vec(), Cipher::from_crypt_filter(&filter));
                }
            }
        }

        let default = |key| match dict.get::<Name<'_>>(key) {
            Some(name) => match named.get(name.deref()) {
                Some(Some(cipher)) => Ok(*cipher),
                Some(None) => Err(UnsupportedAlgorithm),
                None => Ok(Cipher::Identity),
            },
            None => Ok(Cipher::Identity),
        };

        Ok(Self {
            strings: default(STR_F)?,
            streams: default(STM_F)?,
            named,
        })
    }
}

struct StandardRev234<'a> {
    revision: u8,
    key_length: usize,
    owner_string: &'a [u8],
    user_string: &'a [u8],
    permissions: u32,
    id: &'a [u8],
    encrypt_metadata: bool,
}

impl StandardRev234<'_> {
    fn file_key(&self, password: &[u8]) -> Result<Vec<u8>, DecryptionError> {
        if !(2..=4).contains(&self.revision) || !(1..=16).contains(&self.key_length) {
            return Err(InvalidEncryption);
        }

        let password = pdf_doc_encode(password).ok_or(PasswordProtected)?;
        let user_key = |password: &[u8]| {
            let key = self.user_password_key(password);
            self.authenticates_user(&key).then_some(key)
        };

        user_key(&password)
            .or_else(|| user_key(&self.user_password_from_owner_password(&password)))
            .ok_or(PasswordProtected)
    }

    fn user_password_key(&self, password: &[u8]) -> Vec<u8> {
        let mut input = pad_password(password).to_vec();
        input.extend(self.owner_string);
        input.extend(self.permissions.to_le_bytes());
        input.extend(self.id);

        if self.revision >= 4 && !self.encrypt_metadata {
            input.extend([0xff; 4]);
        }

        let mut hash = md5::calculate(&input);

        if self.revision >= 3 {
            for _ in 0..50 {
                hash = md5::calculate(&hash[..self.key_length]);
            }
        }

        hash[..self.key_length].to_vec()
    }

    fn authenticates_user(&self, key: &[u8]) -> bool {
        if self.revision == 2 {
            return rc4_rounds(key, &PASSWORD_PADDING, self.rounds()) == self.user_string;
        }

        let hash = md5::calculate(&[&PASSWORD_PADDING[..], self.id].concat());

        self.user_string.get(..16) == Some(rc4_rounds(key, &hash, self.rounds()).as_slice())
    }

    fn user_password_from_owner_password(&self, owner_password: &[u8]) -> Vec<u8> {
        let mut hash = md5::calculate(&pad_password(owner_password));

        if self.revision >= 3 {
            for _ in 0..50 {
                hash = md5::calculate(&hash);
            }
        }

        let key_length = if self.revision == 2 {
            5
        } else {
            self.key_length
        };

        rc4_rounds(&hash[..key_length], self.owner_string, self.rounds().rev())
    }

    fn rounds(&self) -> RangeInclusive<u8> {
        if self.revision == 2 { 0..=0 } else { 0..=19 }
    }
}

fn file_key_rev56(
    dict: &Dict<'_>,
    revision: u8,
    password: &[u8],
    owner_string: &[u8],
    user_string: &[u8],
) -> Result<Vec<u8>, DecryptionError> {
    let password = &password[..password.len().min(127)];
    let owner_string = owner_string.get(..48).ok_or(InvalidEncryption)?;
    let user_string = user_string.get(..48).ok_or(InvalidEncryption)?;

    let (key_salt, user_key, encrypted_key) = if hash_rev56(
        password,
        &owner_string[32..40],
        user_string,
        revision,
    )? == owner_string[..32]
    {
        (&owner_string[40..], user_string, OE)
    } else if hash_rev56(password, &user_string[32..40], &[], revision)? == user_string[..32] {
        (&user_string[40..], &[][..], UE)
    } else {
        return Err(PasswordProtected);
    };

    let intermediate_key = hash_rev56(password, key_salt, user_key, revision)?;
    let encrypted_key = dict
        .get::<object::String<'_>>(encrypted_key)
        .ok_or(InvalidEncryption)?;

    if encrypted_key.len() != 32 {
        return Err(InvalidEncryption);
    }

    let cipher = AES256Cipher::new(&intermediate_key).ok_or(InvalidEncryption)?;

    Ok(cipher.decrypt_cbc(&encrypted_key, &[0; 16], false))
}

fn hash_rev56(
    password: &[u8],
    salt: &[u8],
    user_key: &[u8],
    revision: u8,
) -> Result<[u8; 32], DecryptionError> {
    let hash = sha256::calculate(&[password, salt, user_key].concat());

    if revision == 5 {
        return Ok(hash);
    }

    let mut k = hash.to_vec();

    let mut round = 0_u16;

    loop {
        let k1 = [password, &k, user_key].concat().repeat(64);
        let cipher = AES128Cipher::new(&k[..16]).ok_or(InvalidEncryption)?;
        let mut e = cipher.encrypt_cbc(&k1, k[16..32].try_into().unwrap());
        e.truncate(k1.len());

        k = match u128::from_be_bytes(e[..16].try_into().unwrap()) % 3 {
            0 => sha256::calculate(&e).to_vec(),
            1 => sha384::calculate(&e).to_vec(),
            _ => sha512::calculate(&e).to_vec(),
        };

        round += 1;

        if round >= 64 && u16::from(*e.last().unwrap()) <= round - 32 {
            break;
        }
    }

    Ok(k[..32].try_into().unwrap())
}

fn rc4_rounds(key: &[u8], data: &[u8], rounds: impl Iterator<Item = u8>) -> Vec<u8> {
    let mut data = data.to_vec();

    for round in rounds {
        let key = key.iter().map(|byte| byte ^ round).collect::<Vec<_>>();
        data = Rc4::new(&key).decrypt(&data);
    }

    data
}

const PASSWORD_PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

fn pad_password(password: &[u8]) -> [u8; 32] {
    let mut padded_password = [0_u8; 32];
    let copy_len = password.len().min(32);
    padded_password[..copy_len].copy_from_slice(&password[..copy_len]);
    padded_password[copy_len..].copy_from_slice(&PASSWORD_PADDING[..(32 - copy_len)]);

    padded_password
}

fn pdf_doc_encode(password: &[u8]) -> Option<Vec<u8>> {
    core::str::from_utf8(password)
        .ok()?
        .chars()
        .map(pdf_doc_byte)
        .collect()
}

fn pdf_doc_byte(c: char) -> Option<u8> {
    const AT_0X18: [char; 8] = [
        '\u{02D8}', '\u{02C7}', '\u{02C6}', '\u{02D9}', '\u{02DD}', '\u{02DB}', '\u{02DA}',
        '\u{02DC}',
    ];
    const AT_0X80: [char; 31] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}',
        '\u{2044}', '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}',
        '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}',
        '\u{0141}', '\u{0152}', '\u{0160}', '\u{0178}', '\u{017D}', '\u{0131}', '\u{0142}',
        '\u{0153}', '\u{0161}', '\u{017E}',
    ];

    match c {
        '\t' | '\n' | '\r' | ' '..='~' | '\u{A1}'..='\u{AC}' | '\u{AE}'..='\u{FF}' => Some(c as u8),
        '\u{20AC}' => Some(0xA0),
        _ => AT_0X18
            .iter()
            .position(|&b| b == c)
            .map(|i| 0x18 + i as u8)
            .or_else(|| AT_0X80.iter().position(|&b| b == c).map(|i| 0x80 + i as u8)),
    }
}
