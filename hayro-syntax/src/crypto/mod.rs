use std::ops::Deref;
use log::warn;
use crate::crypto::algo::Algorithm;
use crate::object::{Dict, Name};
use crate::object::dict::keys::{FILTER, LENGTH, R, V};

mod algo;
mod rc4;
mod md5;

pub(crate) struct CryptoDict {
    algorithm: Algorithm
}

pub(crate) fn get(dict: &Dict) -> Option<Algorithm> {
    let filter = dict.get::<Name>(FILTER)?;
    
    if filter.deref() != b"Standard" {
        warn!("Non-standard encryption filter {} is unsupported", filter.as_str());
        
        return None;
    }
    
    let encryption_v = dict.get::<u8>(V)?;
    let revision = dict.get::<u8>(R)?;
    let algorithm = match encryption_v {
        1 => {
            Algorithm::Rc4
        }
        2 => {
            let length = dict.get::<u32>(LENGTH).unwrap_or(40);
            
            Algorithm::Rc4
        }
        _ => {
            warn!("Unsupported encryption method with number {encryption_v}");
            
            return None;
        }
    };
    
    Some(algorithm)
}