//! A small example that shows how to tokenize a PostScript program.

#![allow(missing_docs)]

use hayro_postscript::{Number, Scanner, Token};
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

    while !scanner.at_end() {
        match scanner.parse_token() {
            Ok(token) => {
                print_token(&token);
                println!();
            }
            Err(e) => {
                eprintln!("Error: {e}");
                break;
            }
        }
    }
}

fn print_token(token: &Token<'_>) {
    match token {
        Token::Number(Number::Integer(n)) => print!("Integer({n})"),
        Token::Number(Number::Real(n)) => print!("Real({n})"),
        Token::Name(name) => {
            let kind = if name.is_literal() {
                "literal"
            } else if name.is_immediate() {
                "immediate"
            } else {
                "executable"
            };
            let text = name.as_str().unwrap_or("<non-ascii name>");
            print!("Name({text}, {kind})");
        }
        Token::String(s) => {
            let decoded = s.decode().unwrap_or_else(|_| Vec::new());
            print!("String({})", String::from_utf8_lossy(&decoded));
        }
        Token::Procedure(proc) => print_procedure(proc.tokens()),
    }
}

fn print_procedure(mut inner: Scanner<'_>) {
    print!("{{");
    let mut first = true;
    while !inner.at_end() {
        if !first {
            print!(" ");
        }
        first = false;
        match inner.parse_token() {
            Ok(token) => print_token(&token),
            Err(e) => {
                print!("Error({e})");
                break;
            }
        }
    }
    print!("}}");
}
