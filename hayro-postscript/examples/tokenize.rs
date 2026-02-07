#![allow(missing_docs)]

use hayro_postscript::{Lexer, Object};
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

    for result in Lexer::new(&data) {
        match result {
            Ok(object) => match object {
                Object::Integer(n) => println!("Integer({n})"),
                Object::Name(ref name) => {
                    let kind = if name.is_literal() { "literal" } else { "executable" };
                    println!("Name({}, {})", lossy(name.data()), kind);
                }
                Object::String(s) => {
                    let decoded = s.decode().unwrap_or_default();
                    match s {
                        hayro_postscript::String::Literal(_) => {
                            println!("String({})", lossy(&decoded));
                        }
                        hayro_postscript::String::Hex(_) => {
                            println!("HexString({})", hex(&decoded));
                        }
                        hayro_postscript::String::Ascii85(_) => {
                            println!("Ascii85String({})", hex(&decoded));
                        }
                    }
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<String>()
}
