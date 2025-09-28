use hayro::Pdf;
use memchr::memmem::Finder;
use rayon::prelude::*;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use walkdir::WalkDir;

static IGNORE_LIST: &[&str] = &[
    // Password-protected
    "0000300", // Broken PDF, maybe fixable
    "0000399", // HTML
    "0000819", "0000920", "0001589", "0002064", "0002187", "0002244", "0002372", "0002554",
    "0002638", "0002966", "0003269", "0003892", "0003927", "0003983", "0004537", "0004889",
    "0004997", "0006169", "0006207", "0006339", "0006844", "0008443", "0008674", "0008978",
    "0009309", "0009464", "0009706", "0010117", "0010216", "0010902",
];

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

    let entries = Mutex::new(vec![]);

    pdf_paths.par_iter().for_each(|path| {
        let data = Arc::new(fs::read(path).unwrap());
        let name = path.file_stem().unwrap().to_str().unwrap().to_string();

        if IGNORE_LIST.contains(&name.as_str()) {
            return;
        }

        match Pdf::new(data.clone()) {
            Ok(_) => {}
            Err(e) => {
                let finder = Finder::new("html");
                let reason = if finder.find(data.as_slice()).is_some() {
                    "html".to_string()
                } else {
                    format!("{:?}", e)
                };
                entries.lock().unwrap().push((name, reason));
            }
        }
    });

    let mut inner = Mutex::into_inner(entries).unwrap();
    inner.sort_by(|(a, _), (b, _)| a.cmp(b));

    for entry in inner {
        println!("{} - {}", entry.0, entry.1);
    }
}
