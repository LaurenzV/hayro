#![allow(missing_docs)]

use hayro_postscript::{Number, Object, Scanner};
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

    let mut scanner = Scanner::new(&data);
    loop {
        match scanner.next() {
            Ok(Some(object)) => {
                print_object(&object);
                println!();
            }
            Ok(None) => break,
            Err(e) => eprintln!("Error: {e}"),
        }
    }
}

fn print_object(object: &Object<'_>) {
    match object {
        Object::Number(Number::Integer(n)) => print!("Integer({n})"),
        Object::Number(Number::Real(n)) => print!("Real({n})"),
        Object::Name(name) => {
            let kind = if name.is_literal() {
                "literal"
            } else {
                "executable"
            };
            let text = name.as_str().unwrap_or("<non-ascii name>");
            print!("Name({text}, {kind})");
        }
        Object::String(s) => {
            let decoded = s.decode().unwrap_or_else(|_| Vec::new());
            print!("String({})", lossy(&decoded));
        }
        Object::Array(arr) => {
            print!("[");
            let mut inner = arr.objects();
            let mut first = true;
            loop {
                match inner.next() {
                    Ok(Some(obj)) => {
                        if !first {
                            print!(" ");
                        }
                        first = false;
                        print_object(&obj);
                    }
                    Ok(None) => break,
                    Err(e) => {
                        if !first {
                            print!(" ");
                        }
                        first = false;
                        print!("Error({e})");
                    }
                }
            }
            print!("]");
        }
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
