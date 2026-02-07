#![allow(missing_docs)]

use hayro_postscript::{Lexer, Object, StringKind};
use std::env;
use std::fs;
use std::process;

fn main() {
    let path = match env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("Usage: tokenize <file>");
            process::exit(1);
        }
    };

    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading {path}: {e}");
            process::exit(1);
        }
    };

    for object in Lexer::new(&data) {
        match object {
            Object::Integer(n) => println!("Integer({n})"),
            Object::Name(ref b) => println!("Name({})", lossy(b)),
            Object::String(sk) => match sk {
                StringKind::Literal(_) => {
                    let decoded = sk.decode().unwrap_or_default();
                    println!("String({})", lossy(&decoded));
                }
                StringKind::Hex(_) => {
                    let decoded = sk.decode().unwrap_or_default();
                    println!("HexString({})", hex(&decoded));
                }
                StringKind::Ascii85(_) => {
                    let decoded = sk.decode().unwrap_or_default();
                    println!("Ascii85String({})", hex(&decoded));
                }
            },
            Object::Operator(ref b) => println!("Operator({})", lossy(b)),
        }
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<String>()
}
