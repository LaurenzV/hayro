#![allow(missing_docs)]

use hayro_postscript::{Scanner, Object};
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

    for result in Scanner::new(&data) {
        match result {
            Ok(object) => match object {
                Object::Integer(n) => println!("Integer({n})"),
                Object::Real(n) => println!("Real({n})"),
                Object::Name(ref name) => {
                    let kind = if name.is_literal() { "literal" } else { "executable" };
                    let text = name.as_str().unwrap_or("<non-ascii name>");
                    println!("Name({text}, {kind})");
                }
                Object::String(s) => {
                    let decoded = s.decode().unwrap_or_else(|_| Vec::new());
                    println!("String({})", lossy(&decoded));
                }
                Object::Array(ref arr) => {
                    println!("Array({} bytes)", arr.data().len());
                }
            },
            Err(e) => {
                eprintln!("Error: {e}");
            }
        }
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
