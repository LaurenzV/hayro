use hayro::Pdf;
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
        .filter(|path| {
            path.extension()
                .unwrap_or_default()
                .eq_ignore_ascii_case("pdf")
        })
        .collect();

    pdf_paths.sort();

    println!("Found {} PDF files", pdf_paths.len());

    pdf_paths.par_iter().for_each(|path| {
        let data = Arc::new(fs::read(path).unwrap());
        match Pdf::new(data) {
            Ok(_) => {}
            Err(e) => println!("  ✗ Failed to load PDF {path:?}: {e:?}"),
        }
    });
}
