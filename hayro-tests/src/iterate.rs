use hayro::{InterpreterSettings, Pdf, render_pdf};
use rayon::prelude::*;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use walkdir::WalkDir;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} <folder>", args[0]);
        std::process::exit(1);
    }

    let folder = &args[1];

    let mut pdf_paths: Vec<PathBuf> = WalkDir::new(folder)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| path.extension().unwrap_or_default().to_ascii_lowercase() == "pdf")
        .collect();

    pdf_paths.sort();

    println!("Found {} PDF files", pdf_paths.len());

    pdf_paths.par_iter().for_each(|path| {
        let data = Arc::new(fs::read(&path).unwrap());
        match Pdf::new(data) {
            Ok(_) => {
                // println!("  ✓ Successfully loaded PDF");
                // match render_pdf(&pdf, 1.0, InterpreterSettings::default(), None) {
                //     Some(_) => println!("  ✓ Successfully rendered PDF"),
                //     None => println!("  ✗ Failed to render PDF"),
                // }
            }
            Err(e) => println!("  ✗ Failed to load PDF {path:?}: {:?}", e),
        }
    });
}
